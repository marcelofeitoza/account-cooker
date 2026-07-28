use std::{
    fmt,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};

use chrono::{DateTime, Utc};
use cooker_core::{
    ActionId, ActionKind, ActionLease, ActionPayload, ActionState, AgentId, AgentSnapshot,
    BudgetUsage, ChainReceipt, ConfirmationStatus, CookerError, LeaseId, PlannedAction,
    PreparedAction, RunId, SessionState, SimulationReceipt, StateExpectation, StateStore,
    TraceEvent,
};
use rusqlite::{
    Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior, params,
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use uuid::Uuid;

use crate::{
    ActionEventRecord, ConfirmationAuditOutcome, ConfirmationAuditRecord, ExpiredLeaseCandidate,
    RecoveryActionCandidate, RecoveryPreview, RecoveryPreviewCounts, RecoveryRecord, RunRecord,
    RunRegistration, SimulationRecord, StoreIdentity, StoreStatusSnapshot, SubmissionRecord,
    migration::{self, sqlite_error, timestamp},
};

const ACTIVE_RUN: &str = "active";
type ExistingRunMetadata = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// `SQLite`-backed durable state store.
pub struct Store {
    path: PathBuf,
    connection: Mutex<Connection>,
    identity: StoreIdentity,
    database_id: Uuid,
}

impl fmt::Debug for Store {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Store")
            .field("path", &self.path)
            .field("identity", &self.identity)
            .field("database_id", &self.database_id)
            .finish_non_exhaustive()
    }
}

impl Store {
    /// Open or create a store and apply all embedded migrations.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid identity, incompatible database, failed
    /// migration, failed integrity check, or filesystem/`SQLite` failure.
    pub fn open(path: impl AsRef<Path>, identity: StoreIdentity) -> Result<Self, CookerError> {
        identity.validate()?;
        let path = path.as_ref().to_path_buf();
        let mut connection =
            Connection::open(&path).map_err(|error| sqlite_error("open SQLite database", error))?;
        migration::configure(&connection)?;
        let database_id = migration::initialize(&mut connection, &identity)?;
        Ok(Self {
            path,
            connection: Mutex::new(connection),
            identity,
            database_id,
        })
    }

    /// Open an existing, fully migrated store without creating or changing database state.
    ///
    /// # Errors
    ///
    /// Returns an error when the file is missing, cannot be opened read-only, has an outdated or
    /// inconsistent migration journal, fails integrity checks, or has a different identity.
    pub fn open_read_only(
        path: impl AsRef<Path>,
        identity: StoreIdentity,
    ) -> Result<Self, CookerError> {
        identity.validate()?;
        let path = path.as_ref().to_path_buf();
        let metadata = std::fs::symlink_metadata(&path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                CookerError::NotFound(format!("runtime database {}", path.display()))
            } else {
                CookerError::Store(format!(
                    "cannot inspect runtime database {}: {error}",
                    path.display()
                ))
            }
        })?;
        if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
            return Err(CookerError::NotFound(format!(
                "regular non-symlink runtime database {}",
                path.display()
            )));
        }
        let wal = sqlite_sidecar(&path, "-wal");
        let shared_memory = sqlite_sidecar(&path, "-shm");
        let sidecars = (wal.exists(), shared_memory.exists());
        if matches!(sidecars, (true, false) | (false, true)) {
            return Err(CookerError::Store(
                "read-only WAL database has an incomplete sidecar pair".to_owned(),
            ));
        }
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let connection = if sidecars == (true, true) {
            Connection::open_with_flags(&path, flags)
        } else {
            let mut uri = url::Url::from_file_path(&path).map_err(|()| {
                CookerError::InvalidConfig(format!(
                    "runtime database path cannot be represented as a file URI: {}",
                    path.display()
                ))
            })?;
            uri.set_query(Some("immutable=1"));
            Connection::open_with_flags(uri.as_str(), flags | OpenFlags::SQLITE_OPEN_URI)
        }
        .map_err(|error| sqlite_error("open SQLite database read-only", error))?;
        migration::configure_read_only(&connection, sidecars == (true, true))?;
        let database_id = migration::verify_current(&connection, &identity)?;
        Ok(Self {
            path,
            connection: Mutex::new(connection),
            identity,
            database_id,
        })
    }

    /// Return the identity this process required when opening the database.
    #[must_use]
    pub const fn identity(&self) -> &StoreIdentity {
        &self.identity
    }

    /// Return the stable random identity assigned to this database file.
    #[must_use]
    pub const fn database_id(&self) -> Uuid {
        self.database_id
    }

    /// Return the latest applied embedded migration version.
    ///
    /// # Errors
    ///
    /// Returns an error when the connection cannot be locked or queried.
    pub fn schema_version(&self) -> Result<u32, CookerError> {
        let connection = self.lock()?;
        connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(|error| sqlite_error("read schema version", error))
    }

    /// Return `SQLite`'s active journal mode.
    ///
    /// # Errors
    ///
    /// Returns an error when the connection cannot be locked or queried.
    pub fn journal_mode(&self) -> Result<String, CookerError> {
        let connection = self.lock()?;
        connection
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .map_err(|error| sqlite_error("read journal mode", error))
    }

    /// Re-run `SQLite` application, page, and foreign-key integrity checks.
    ///
    /// # Errors
    ///
    /// Returns an error when any integrity check fails.
    pub fn verify_integrity(&self) -> Result<(), CookerError> {
        let connection = self.lock()?;
        migration::verify(&connection)
    }

    /// Register full run metadata, enriching an automatically created stub when needed.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid metadata, conflicting registration, or a
    /// storage failure.
    pub fn register_run(&self, registration: &RunRegistration) -> Result<bool, CookerError> {
        validate_nonempty("model_version", &registration.model_version)?;
        validate_hash("config_hash", &registration.config_hash)?;
        validate_hash("seed_hash", &registration.seed_hash)?;
        let config_json = encode_json(&registration.config)?;
        let mut connection = self.lock()?;
        let transaction = immediate(&mut connection, "register run")?;
        let existing: Option<ExistingRunMetadata> = transaction
            .query_row(
                "SELECT model_version, config_json, config_hash, seed_hash \
                     FROM runs WHERE run_id = ?1",
                [registration.id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()
            .map_err(|error| sqlite_error("load existing run", error))?;

        let inserted = match existing {
            None => {
                transaction
                    .execute(
                        "INSERT INTO runs(\
                            run_id, status, model_version, config_json, config_hash, seed_hash,\
                            created_at, updated_at\
                         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                        params![
                            registration.id.to_string(),
                            ACTIVE_RUN,
                            registration.model_version,
                            config_json,
                            registration.config_hash,
                            registration.seed_hash,
                            timestamp(registration.created_at)
                        ],
                    )
                    .map_err(|error| sqlite_error("insert run", error))?;
                true
            }
            Some((None, None, None, None)) => {
                transaction
                    .execute(
                        "UPDATE runs SET model_version = ?2, config_json = ?3, \
                            config_hash = ?4, seed_hash = ?5, updated_at = ?6 \
                         WHERE run_id = ?1",
                        params![
                            registration.id.to_string(),
                            registration.model_version,
                            config_json,
                            registration.config_hash,
                            registration.seed_hash,
                            timestamp(registration.created_at)
                        ],
                    )
                    .map_err(|error| sqlite_error("enrich run stub", error))?;
                true
            }
            Some((model, config, config_hash, seed_hash)) => {
                if model.as_deref() != Some(registration.model_version.as_str())
                    || config.as_deref() != Some(config_json.as_str())
                    || config_hash.as_deref() != Some(registration.config_hash.as_str())
                    || seed_hash.as_deref() != Some(registration.seed_hash.as_str())
                {
                    return Err(CookerError::Store(format!(
                        "run {} was registered with conflicting metadata",
                        registration.id
                    )));
                }
                false
            }
        };
        transaction
            .commit()
            .map_err(|error| sqlite_error("commit run registration", error))?;
        Ok(inserted)
    }

    /// Load persisted run metadata.
    ///
    /// # Errors
    ///
    /// Returns an error when the run is missing or persisted data is invalid.
    pub fn get_run(&self, run_id: RunId) -> Result<RunRecord, CookerError> {
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT status, model_version, config_json, config_hash, seed_hash, \
                        created_at, updated_at \
                 FROM runs WHERE run_id = ?1",
                [run_id.to_string()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| sqlite_error("load run", error))?
            .map_or_else(
                || Err(CookerError::NotFound(format!("run {run_id}"))),
                |(status, model_version, config, config_hash, seed_hash, created, updated)| {
                    Ok(RunRecord {
                        id: run_id,
                        status,
                        model_version,
                        config: config.map(|json| decode_json(&json)).transpose()?,
                        config_hash,
                        seed_hash,
                        created_at: parse_timestamp(&created)?,
                        updated_at: parse_timestamp(&updated)?,
                    })
                },
            )
    }

    /// Compare-and-swap a run status.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid states, a failed comparison, or storage failure.
    pub fn transition_run(
        &self,
        run_id: RunId,
        expected: &str,
        next: &str,
        at: DateTime<Utc>,
    ) -> Result<(), CookerError> {
        validate_run_status(expected)?;
        validate_run_status(next)?;
        let connection = self.lock()?;
        let changed = connection
            .execute(
                "UPDATE runs SET status = ?3, updated_at = ?4 \
                 WHERE run_id = ?1 AND status = ?2",
                params![run_id.to_string(), expected, next, timestamp(at)],
            )
            .map_err(|error| sqlite_error("transition run", error))?;
        if changed != 1 {
            return Err(CookerError::Store(format!(
                "run {run_id} status compare-and-swap failed from {expected} to {next}"
            )));
        }
        Ok(())
    }

    /// Insert or update an agent snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid data, conflicting ownership, or storage failure.
    pub fn upsert_agent(
        &self,
        snapshot: &AgentSnapshot,
        at: DateTime<Utc>,
    ) -> Result<(), CookerError> {
        validate_nonempty("agent model_version", &snapshot.model_version)?;
        let next_sequence = to_i64(snapshot.next_sequence, "agent next_sequence")?;
        let remaining = to_i64(
            snapshot.remaining_daily_budget,
            "agent remaining_daily_budget",
        )?;
        let snapshot_json = encode_json(snapshot)?;
        let mut connection = self.lock()?;
        let transaction = immediate(&mut connection, "upsert agent")?;
        ensure_run_stub(&transaction, snapshot.run_id, at)?;
        let existing_run: Option<String> = transaction
            .query_row(
                "SELECT run_id FROM agents WHERE agent_id = ?1",
                [snapshot.id.to_string()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| sqlite_error("load agent ownership", error))?;
        if existing_run
            .as_deref()
            .is_some_and(|run| run != snapshot.run_id.to_string())
        {
            return Err(CookerError::Store(format!(
                "agent {} belongs to a different run",
                snapshot.id
            )));
        }
        transaction
            .execute(
                "INSERT INTO agents(\
                    agent_id, run_id, next_sequence, next_decision_at, budget_date, session_state,\
                    last_action_at, remaining_daily_budget, model_version, snapshot_json,\
                    created_at, updated_at\
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11) \
                 ON CONFLICT(agent_id) DO UPDATE SET \
                    next_sequence = excluded.next_sequence, \
                    next_decision_at = excluded.next_decision_at, \
                    budget_date = excluded.budget_date, \
                    session_state = excluded.session_state, \
                    last_action_at = excluded.last_action_at, \
                    remaining_daily_budget = excluded.remaining_daily_budget, \
                    model_version = excluded.model_version, \
                    snapshot_json = excluded.snapshot_json, \
                    updated_at = excluded.updated_at",
                params![
                    snapshot.id.to_string(),
                    snapshot.run_id.to_string(),
                    next_sequence,
                    timestamp(snapshot.next_decision_at),
                    snapshot.budget_date.to_string(),
                    session_state_name(snapshot.session_state),
                    snapshot.last_action_at.map(timestamp),
                    remaining,
                    snapshot.model_version,
                    snapshot_json,
                    timestamp(at)
                ],
            )
            .map_err(|error| sqlite_error("upsert agent", error))?;
        transaction
            .commit()
            .map_err(|error| sqlite_error("commit agent upsert", error))?;
        Ok(())
    }

    /// Load an agent snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error when the agent is missing or persisted data is invalid.
    pub fn get_agent(&self, agent_id: AgentId) -> Result<AgentSnapshot, CookerError> {
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT snapshot_json FROM agents WHERE agent_id = ?1",
                [agent_id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| sqlite_error("load agent", error))?
            .map_or_else(
                || Err(CookerError::NotFound(format!("agent {agent_id}"))),
                |json| decode_json(&json),
            )
    }

    /// Load due planner snapshots in stable deadline and identity order.
    ///
    /// # Errors
    ///
    /// Returns an error when the limit is zero or snapshots cannot be queried and decoded.
    pub fn due_agents(
        &self,
        run_id: RunId,
        now: DateTime<Utc>,
        limit: usize,
    ) -> Result<Vec<AgentSnapshot>, CookerError> {
        if limit == 0 {
            return Err(CookerError::InvalidConfig(
                "due agent limit must be positive".to_owned(),
            ));
        }
        let limit = i64::try_from(limit).map_err(|error| {
            CookerError::InvalidConfig(format!("due agent limit does not fit SQLite: {error}"))
        })?;
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT snapshot_json FROM agents \
                 WHERE run_id = ?1 AND next_decision_at <= ?2 \
                 ORDER BY next_decision_at, agent_id LIMIT ?3",
            )
            .map_err(|error| sqlite_error("prepare due agent query", error))?;
        let rows = statement
            .query_map(params![run_id.to_string(), timestamp(now), limit], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|error| sqlite_error("query due agents", error))?;
        let mut snapshots = Vec::new();
        for row in rows {
            let json = row.map_err(|error| sqlite_error("read due agent", error))?;
            snapshots.push(decode_json(&json)?);
        }
        Ok(snapshots)
    }

    /// Compare-and-swap an initialized agent's planner snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid sequence advance, missing agent, or storage failure.
    pub fn advance_agent(
        &self,
        expected_sequence: u64,
        snapshot: &AgentSnapshot,
        at: DateTime<Utc>,
    ) -> Result<bool, CookerError> {
        if snapshot.next_sequence
            != expected_sequence.checked_add(1).ok_or_else(|| {
                CookerError::InvalidConfig("agent sequence advance overflow".to_owned())
            })?
        {
            return Err(CookerError::InvalidConfig(
                "agent snapshot must advance exactly one sequence".to_owned(),
            ));
        }
        validate_nonempty("agent model_version", &snapshot.model_version)?;
        let expected = to_i64(expected_sequence, "expected agent sequence")?;
        let next = to_i64(snapshot.next_sequence, "agent next sequence")?;
        let remaining = to_i64(
            snapshot.remaining_daily_budget,
            "agent remaining daily budget",
        )?;
        let snapshot_json = encode_json(snapshot)?;
        let mut connection = self.lock()?;
        let transaction = immediate(&mut connection, "advance agent")?;
        let changed = transaction
            .execute(
                "UPDATE agents SET \
                    next_sequence = ?4, next_decision_at = ?5, budget_date = ?6,\
                    session_state = ?7, last_action_at = ?8, remaining_daily_budget = ?9,\
                    model_version = ?10, snapshot_json = ?11, updated_at = ?12 \
                 WHERE agent_id = ?1 AND run_id = ?2 AND next_sequence = ?3",
                params![
                    snapshot.id.to_string(),
                    snapshot.run_id.to_string(),
                    expected,
                    next,
                    timestamp(snapshot.next_decision_at),
                    snapshot.budget_date.to_string(),
                    session_state_name(snapshot.session_state),
                    snapshot.last_action_at.map(timestamp),
                    remaining,
                    snapshot.model_version,
                    snapshot_json,
                    timestamp(at)
                ],
            )
            .map_err(|error| sqlite_error("compare-and-swap agent snapshot", error))?;
        if changed == 0 {
            let exists: bool = transaction
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM agents WHERE agent_id = ?1)",
                    [snapshot.id.to_string()],
                    |row| row.get(0),
                )
                .map_err(|error| sqlite_error("check advanced agent", error))?;
            if !exists {
                return Err(CookerError::NotFound(format!("agent {}", snapshot.id)));
            }
        }
        transaction
            .commit()
            .map_err(|error| sqlite_error("commit agent advance", error))?;
        Ok(changed == 1)
    }

    /// Load the current lifecycle state of an action.
    ///
    /// # Errors
    ///
    /// Returns an error when the action is missing or persisted state is invalid.
    pub fn action_state(&self, action_id: &ActionId) -> Result<ActionState, CookerError> {
        let connection = self.lock()?;
        load_action_state(&connection, action_id)
    }

    /// Load all durable material needed to continue an action after restart.
    ///
    /// # Errors
    ///
    /// Returns an error when the action is missing or any persisted artifact is invalid.
    pub fn recovery_record(
        &self,
        action_id: &ActionId,
        now: DateTime<Utc>,
    ) -> Result<RecoveryRecord, CookerError> {
        let connection = self.lock()?;
        let (action, state) = load_action_and_state(&connection, action_id)?;
        let prepared = load_prepared(&connection, action_id)?;
        let simulations = load_simulations(&connection, action_id)?;
        let submission = load_submission(&connection, action_id)?;
        let receipts = load_receipts(&connection, action_id)?;
        let active_lease = load_active_lease(&connection, action_id, now)?;
        let events = load_events(&connection, action_id)?;
        Ok(RecoveryRecord {
            action,
            state,
            prepared,
            simulations,
            submission,
            receipts,
            active_lease,
            events,
        })
    }

    /// Load an action's immutable event journal in insertion order.
    ///
    /// # Errors
    ///
    /// Returns an error when the journal cannot be queried or decoded.
    pub fn action_events(
        &self,
        action_id: &ActionId,
    ) -> Result<Vec<ActionEventRecord>, CookerError> {
        let connection = self.lock()?;
        load_events(&connection, action_id)
    }

    /// Load evaluator traces for one run in append order.
    ///
    /// # Errors
    ///
    /// Returns an error when traces cannot be queried or decoded.
    pub fn traces_for_run(&self, run_id: RunId) -> Result<Vec<TraceEvent>, CookerError> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare("SELECT event_json FROM traces WHERE run_id = ?1 ORDER BY trace_id")
            .map_err(|error| sqlite_error("prepare trace query", error))?;
        let rows = statement
            .query_map([run_id.to_string()], |row| row.get::<_, String>(0))
            .map_err(|error| sqlite_error("query traces", error))?;
        let mut traces = Vec::new();
        for row in rows {
            let json = row.map_err(|error| sqlite_error("read trace", error))?;
            traces.push(decode_json(&json)?);
        }
        Ok(traces)
    }

    /// Return immutable confirmation-audit history in persistence order.
    ///
    /// # Errors
    ///
    /// Returns an error when the action is unknown or history cannot be decoded.
    pub fn confirmation_audits(
        &self,
        action_id: &ActionId,
    ) -> Result<Vec<ConfirmationAuditRecord>, CookerError> {
        let connection = self.lock()?;
        load_action_state(&connection, action_id)?;
        let mut statement = connection
            .prepare(
                "SELECT audit_id, receipt_json, audited_at, outcome \
                 FROM confirmation_audits WHERE action_id = ?1 ORDER BY audit_id",
            )
            .map_err(|error| sqlite_error("prepare confirmation audit history", error))?;
        let rows = statement
            .query_map([action_id.as_str()], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(|error| sqlite_error("query confirmation audit history", error))?;
        let mut audits = Vec::new();
        for row in rows {
            let (id, receipt, audited_at, outcome) =
                row.map_err(|error| sqlite_error("read confirmation audit", error))?;
            audits.push(ConfirmationAuditRecord {
                id,
                receipt: decode_json(&receipt)?,
                audited_at: parse_timestamp(&audited_at)?,
                outcome: parse_audit_outcome(&outcome)?,
            });
        }
        Ok(audits)
    }

    /// Return a serializable, read-only status snapshot evaluated at an explicit instant.
    ///
    /// Pass an audit cutoff to count confirmed actions whose latest confirmation observation is
    /// strictly older than that cutoff. Passing `None` reports that audit scheduling is not
    /// configured instead of conflating it with a zero count.
    ///
    /// # Errors
    ///
    /// Returns an error when the audit cutoff is later than `now` or any count cannot be queried
    /// or decoded.
    pub fn status_snapshot(
        &self,
        now: DateTime<Utc>,
        confirmation_audit_due_before: Option<DateTime<Utc>>,
    ) -> Result<StoreStatusSnapshot, CookerError> {
        validate_audit_cutoff(now, confirmation_audit_due_before)?;
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|error| sqlite_error("begin status snapshot", error))?;
        let total_agents = query_count(&transaction, "SELECT count(*) FROM agents", [], "agents")?;
        let total_actions =
            query_count(&transaction, "SELECT count(*) FROM actions", [], "actions")?;
        let actions_by_state = load_action_state_counts(&transaction)?;
        let active_leases = query_count(
            &transaction,
            "SELECT count(*) FROM leases \
             WHERE released_at IS NULL AND expires_at > ?1",
            [timestamp(now)],
            "active leases",
        )?;
        let expired_leases = count_expired_leases(&transaction, now)?;
        let due_agents = query_count(
            &transaction,
            "SELECT count(*) FROM agents WHERE next_decision_at <= ?1",
            [timestamp(now)],
            "due agents",
        )?;
        let due_actions = count_due_actions(&transaction, now)?;
        let unresolved_actions = query_count(
            &transaction,
            "SELECT count(*) FROM actions \
             WHERE state IN ('submitted', 'unknown', 'orphaned')",
            [],
            "unresolved actions",
        )?;
        let due_confirmation_audits = confirmation_audit_due_before
            .map(|cutoff| count_due_confirmation_audits(&transaction, now, cutoff))
            .transpose()?;
        let snapshot = StoreStatusSnapshot {
            database_id: self.database_id,
            identity: self.identity.clone(),
            observed_at: now,
            total_agents,
            total_actions,
            actions_by_state,
            active_leases,
            expired_leases,
            due_agents,
            due_actions,
            unresolved_actions,
            due_confirmation_audits,
        };
        transaction
            .commit()
            .map_err(|error| sqlite_error("commit status snapshot", error))?;
        Ok(snapshot)
    }

    /// Preview actionable recovery work without releasing leases or claiming actions.
    ///
    /// Counts always cover all matching rows. Each deterministic list is independently bounded by
    /// `max_items_per_category`; use zero for a count-only preview. Audit candidates are omitted
    /// and their count is `None` when no audit cutoff is supplied.
    ///
    /// # Errors
    ///
    /// Returns an error when the audit cutoff is later than `now`, the list bound does not fit
    /// `SQLite`, or candidate rows cannot be queried or decoded.
    pub fn recovery_preview(
        &self,
        now: DateTime<Utc>,
        confirmation_audit_due_before: Option<DateTime<Utc>>,
        max_items_per_category: usize,
    ) -> Result<RecoveryPreview, CookerError> {
        validate_audit_cutoff(now, confirmation_audit_due_before)?;
        let limit = i64::try_from(max_items_per_category).map_err(|error| {
            CookerError::InvalidConfig(format!(
                "recovery preview limit does not fit SQLite: {error}"
            ))
        })?;
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|error| sqlite_error("begin recovery preview", error))?;
        let expired_lease_count = count_expired_leases(&transaction, now)?;
        let reconciliation_count = count_reconciliation_candidates(&transaction, now)?;
        let confirmation_audit_count = confirmation_audit_due_before
            .map(|cutoff| count_due_confirmation_audits(&transaction, now, cutoff))
            .transpose()?;
        let expired_leases = load_expired_lease_candidates(&transaction, now, limit)?;
        let reconciliation_candidates = load_reconciliation_candidates(&transaction, now, limit)?;
        let confirmation_audit_candidates = confirmation_audit_due_before.map_or_else(
            || Ok(Vec::new()),
            |cutoff| load_confirmation_audit_candidates(&transaction, now, cutoff, limit),
        )?;
        let preview = RecoveryPreview {
            observed_at: now,
            confirmation_audit_due_before,
            max_items_per_category,
            counts: RecoveryPreviewCounts {
                expired_leases: expired_lease_count,
                reconciliation_candidates: reconciliation_count,
                confirmation_audit_candidates: confirmation_audit_count,
            },
            expired_leases,
            reconciliation_candidates,
            confirmation_audit_candidates,
        };
        transaction
            .commit()
            .map_err(|error| sqlite_error("commit recovery preview", error))?;
        Ok(preview)
    }

    /// Release a lease using an explicit time, primarily for virtual-time runtimes.
    ///
    /// # Errors
    ///
    /// Returns an error when lease identity, ownership, action, or expiry does not match.
    pub fn release_lease_at(
        &self,
        lease: &ActionLease,
        at: DateTime<Utc>,
    ) -> Result<(), CookerError> {
        let mut connection = self.lock()?;
        let transaction = immediate(&mut connection, "release lease")?;
        verify_lease(&transaction, lease, at)?;
        release_lease_in_transaction(&transaction, lease, at, "released")?;
        append_event(
            &transaction,
            &lease.action_id,
            "lease_released",
            None,
            None,
            at,
            Some(&lease.worker_id),
            Some(&serde_json::json!({"lease_id": lease.id.to_string()})),
        )?;
        transaction
            .commit()
            .map_err(|error| sqlite_error("commit lease release", error))?;
        Ok(())
    }

    /// Mark all expired active leases as released and return the count.
    ///
    /// # Errors
    ///
    /// Returns an error when lease expiry cannot be persisted atomically.
    pub fn release_expired_leases(&self, now: DateTime<Utc>) -> Result<usize, CookerError> {
        let mut connection = self.lock()?;
        let transaction = immediate(&mut connection, "expire leases")?;
        let count = expire_leases(&transaction, now)?;
        transaction
            .commit()
            .map_err(|error| sqlite_error("commit expired leases", error))?;
        Ok(count)
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, CookerError> {
        self.connection
            .lock()
            .map_err(|_| CookerError::Store("SQLite connection lock poisoned".to_owned()))
    }

    #[allow(clippy::too_many_lines)]
    fn claim(
        &self,
        worker_id: &str,
        now: DateTime<Utc>,
        lease_until: DateTime<Utc>,
        limit: usize,
        reconciliation: bool,
    ) -> Result<Vec<ActionLease>, CookerError> {
        validate_claim(worker_id, now, lease_until, limit)?;
        let sql_limit = i64::try_from(limit).map_err(|error| {
            CookerError::InvalidConfig(format!("claim limit does not fit SQLite: {error}"))
        })?;
        let mut connection = self.lock()?;
        let transaction = immediate(&mut connection, "claim actions")?;
        expire_leases(&transaction, now)?;

        let query = if reconciliation {
            "SELECT a.action_id FROM actions a \
             WHERE ( \
                 a.state IN ('submitted', 'unknown', 'orphaned') \
                 OR (a.state = 'simulated' AND EXISTS ( \
                     SELECT 1 FROM submissions s WHERE s.action_id = a.action_id \
                 )) \
             ) \
             AND NOT EXISTS ( \
                 SELECT 1 FROM leases l \
                 WHERE l.action_id = a.action_id AND l.released_at IS NULL \
             ) \
             ORDER BY a.updated_at, a.action_id LIMIT ?1"
        } else {
            "SELECT a.action_id FROM actions a \
             LEFT JOIN action_deferrals d ON d.action_id = a.action_id \
             WHERE COALESCE(d.eligible_at, a.scheduled_at) <= ?1 \
               AND a.state IN ('planned', 'simulated') \
               AND NOT EXISTS ( \
                   SELECT 1 FROM submissions s WHERE s.action_id = a.action_id \
               ) \
               AND NOT EXISTS ( \
                   SELECT 1 FROM leases l \
                   WHERE l.action_id = a.action_id AND l.released_at IS NULL \
               ) \
             ORDER BY COALESCE(d.eligible_at, a.scheduled_at), a.action_id LIMIT ?2"
        };

        let action_ids = {
            let mut statement = transaction
                .prepare(query)
                .map_err(|error| sqlite_error("prepare claim query", error))?;
            let mut ids = Vec::new();
            if reconciliation {
                let rows = statement
                    .query_map([sql_limit], |row| row.get::<_, String>(0))
                    .map_err(|error| sqlite_error("query reconciliation claims", error))?;
                for row in rows {
                    ids.push(row.map_err(|error| sqlite_error("read claim candidate", error))?);
                }
            } else {
                let rows = statement
                    .query_map(params![timestamp(now), sql_limit], |row| {
                        row.get::<_, String>(0)
                    })
                    .map_err(|error| sqlite_error("query due claims", error))?;
                for row in rows {
                    ids.push(row.map_err(|error| sqlite_error("read claim candidate", error))?);
                }
            }
            ids
        };

        let mut leases = Vec::with_capacity(action_ids.len());
        for action_id_text in action_ids {
            let action_id = decode_action_id(&action_id_text)?;
            let lease = ActionLease {
                id: LeaseId::new(),
                action_id,
                worker_id: worker_id.to_owned(),
                expires_at: lease_until,
            };
            transaction
                .execute(
                    "INSERT INTO leases(\
                        lease_id, action_id, worker_id, acquired_at, expires_at\
                     ) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        lease.id.to_string(),
                        lease.action_id.as_str(),
                        lease.worker_id,
                        timestamp(now),
                        timestamp(lease_until)
                    ],
                )
                .map_err(|error| sqlite_error("insert action lease", error))?;
            append_event(
                &transaction,
                &lease.action_id,
                if reconciliation {
                    "reconciliation_lease_acquired"
                } else {
                    "lease_acquired"
                },
                None,
                None,
                now,
                Some(worker_id),
                Some(&serde_json::json!({
                    "lease_id": lease.id.to_string(),
                    "expires_at": timestamp(lease_until),
                })),
            )?;
            leases.push(lease);
        }
        transaction
            .commit()
            .map_err(|error| sqlite_error("commit action claims", error))?;
        Ok(leases)
    }

    fn claim_confirmation_audits(
        &self,
        worker_id: &str,
        now: DateTime<Utc>,
        lease_until: DateTime<Utc>,
        due_before: DateTime<Utc>,
        limit: usize,
    ) -> Result<Vec<ActionLease>, CookerError> {
        validate_claim(worker_id, now, lease_until, limit)?;
        if due_before > now {
            return Err(CookerError::InvalidConfig(
                "confirmation audit cutoff cannot be in the future".to_owned(),
            ));
        }
        let sql_limit = i64::try_from(limit).map_err(|error| {
            CookerError::InvalidConfig(format!("claim limit does not fit SQLite: {error}"))
        })?;
        let mut connection = self.lock()?;
        let transaction = immediate(&mut connection, "claim confirmation audits")?;
        expire_leases(&transaction, now)?;
        let action_ids = {
            let mut statement = transaction
                .prepare(
                    "SELECT a.action_id FROM actions a \
                     WHERE a.state = 'confirmed' \
                       AND COALESCE(( \
                           SELECT MAX(ca.audited_at) FROM confirmation_audits ca \
                           WHERE ca.action_id = a.action_id \
                       ), a.updated_at) < ?1 \
                       AND NOT EXISTS ( \
                           SELECT 1 FROM leases l \
                           WHERE l.action_id = a.action_id AND l.released_at IS NULL \
                       ) \
                     ORDER BY COALESCE(( \
                         SELECT MAX(ca.audited_at) FROM confirmation_audits ca \
                         WHERE ca.action_id = a.action_id \
                     ), a.updated_at), a.action_id LIMIT ?2",
                )
                .map_err(|error| sqlite_error("prepare confirmation audit claims", error))?;
            let rows = statement
                .query_map(params![timestamp(due_before), sql_limit], |row| {
                    row.get::<_, String>(0)
                })
                .map_err(|error| sqlite_error("query confirmation audit claims", error))?;
            let mut ids = Vec::new();
            for row in rows {
                ids.push(row.map_err(|error| sqlite_error("read audit candidate", error))?);
            }
            ids
        };

        let mut leases = Vec::with_capacity(action_ids.len());
        for action_id_text in action_ids {
            let lease = ActionLease {
                id: LeaseId::new(),
                action_id: decode_action_id(&action_id_text)?,
                worker_id: worker_id.to_owned(),
                expires_at: lease_until,
            };
            transaction
                .execute(
                    "INSERT INTO leases( \
                        lease_id, action_id, worker_id, acquired_at, expires_at \
                     ) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        lease.id.to_string(),
                        lease.action_id.as_str(),
                        lease.worker_id,
                        timestamp(now),
                        timestamp(lease_until)
                    ],
                )
                .map_err(|error| sqlite_error("insert confirmation audit lease", error))?;
            append_event(
                &transaction,
                &lease.action_id,
                "confirmation_audit_lease_acquired",
                None,
                None,
                now,
                Some(worker_id),
                Some(&serde_json::json!({
                    "lease_id": lease.id.to_string(),
                    "expires_at": timestamp(lease_until),
                    "due_before": timestamp(due_before),
                })),
            )?;
            leases.push(lease);
        }
        transaction
            .commit()
            .map_err(|error| sqlite_error("commit confirmation audit claims", error))?;
        Ok(leases)
    }
}

fn sqlite_sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_owned();
    value.push(suffix);
    PathBuf::from(value)
}

impl StateStore for Store {
    fn upsert_agent(&self, snapshot: &AgentSnapshot, at: DateTime<Utc>) -> Result<(), CookerError> {
        Store::upsert_agent(self, snapshot, at)
    }

    fn get_agent(&self, agent_id: AgentId) -> Result<AgentSnapshot, CookerError> {
        Store::get_agent(self, agent_id)
    }

    fn due_agents(
        &self,
        run_id: RunId,
        now: DateTime<Utc>,
        limit: usize,
    ) -> Result<Vec<AgentSnapshot>, CookerError> {
        Store::due_agents(self, run_id, now, limit)
    }

    fn advance_agent(
        &self,
        expected_sequence: u64,
        snapshot: &AgentSnapshot,
        at: DateTime<Utc>,
    ) -> Result<bool, CookerError> {
        Store::advance_agent(self, expected_sequence, snapshot, at)
    }

    fn enqueue_action(&self, action: &PlannedAction) -> Result<bool, CookerError> {
        validate_action(action)?;
        let action_json = encode_json(action)?;
        let sequence = to_i64(action.sequence, "action sequence")?;
        let max_fee = to_i64(action.max_fee_lamports, "maximum fee")?;
        let max_account_creation = to_i64(
            action.max_account_creation_lamports,
            "maximum account creation funding",
        )?;
        let mut connection = self.lock()?;
        let transaction = immediate(&mut connection, "enqueue action")?;
        ensure_run_stub(&transaction, action.run_id, action.created_at)?;
        ensure_agent_stub(&transaction, action)?;
        let inserted = transaction
            .execute(
                "INSERT OR IGNORE INTO actions(\
                    action_id, run_id, agent_id, sequence, model_version, action_kind,\
                    scheduled_at, max_fee_lamports, max_account_creation_lamports, state,\
                    action_json, created_at, updated_at\
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'planned', ?10, ?11, ?11)",
                params![
                    action.id.as_str(),
                    action.run_id.to_string(),
                    action.agent_id.to_string(),
                    sequence,
                    action.model_version,
                    action_kind_name(action.payload.kind()),
                    timestamp(action.scheduled_at),
                    max_fee,
                    max_account_creation,
                    action_json,
                    timestamp(action.created_at)
                ],
            )
            .map_err(|error| sqlite_error("insert action", error))?;
        if inserted == 0 {
            let existing: Option<String> = transaction
                .query_row(
                    "SELECT action_json FROM actions WHERE action_id = ?1",
                    [action.id.as_str()],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|error| sqlite_error("load duplicate action", error))?;
            if existing
                .as_deref()
                .map(decode_json::<PlannedAction>)
                .transpose()?
                .as_ref()
                != Some(action)
            {
                return Err(CookerError::Store(format!(
                    "deterministic action identity collision for {}",
                    action.id
                )));
            }
            transaction
                .commit()
                .map_err(|error| sqlite_error("commit duplicate action check", error))?;
            return Ok(false);
        }
        append_event(
            &transaction,
            &action.id,
            "planned",
            None,
            Some(ActionState::Planned),
            action.created_at,
            None,
            None,
        )?;
        transaction
            .commit()
            .map_err(|error| sqlite_error("commit action enqueue", error))?;
        Ok(true)
    }

    fn claim_due_actions(
        &self,
        worker_id: &str,
        now: DateTime<Utc>,
        lease_until: DateTime<Utc>,
        limit: usize,
    ) -> Result<Vec<ActionLease>, CookerError> {
        self.claim(worker_id, now, lease_until, limit, false)
    }

    fn get_action(&self, action_id: &ActionId) -> Result<PlannedAction, CookerError> {
        let connection = self.lock()?;
        load_action_and_state(&connection, action_id).map(|(action, _)| action)
    }

    fn get_action_state(&self, action_id: &ActionId) -> Result<ActionState, CookerError> {
        self.action_state(action_id)
    }

    fn get_prepared(&self, action_id: &ActionId) -> Result<Option<PreparedAction>, CookerError> {
        let connection = self.lock()?;
        load_prepared(&connection, action_id)
    }

    fn get_submission_signature(
        &self,
        action_id: &ActionId,
    ) -> Result<Option<String>, CookerError> {
        let connection = self.lock()?;
        load_submission(&connection, action_id)
            .map(|submission| submission.map(|record| record.signature))
    }

    fn budget_usage(
        &self,
        agent_id: AgentId,
        now: DateTime<Utc>,
    ) -> Result<BudgetUsage, CookerError> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT a.action_json, COALESCE(s.submitted_at, a.updated_at) \
                 FROM actions a LEFT JOIN submissions s ON s.action_id = a.action_id \
                 WHERE a.agent_id = ?1 \
                   AND ( \
                       a.state IN ('submitted', 'unknown', 'confirmed') \
                       OR (a.state = 'simulated' AND s.action_id IS NOT NULL) \
                   ) \
                 ORDER BY a.action_id",
            )
            .map_err(|error| sqlite_error("prepare budget usage query", error))?;
        let rows = statement
            .query_map([agent_id.to_string()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| sqlite_error("query budget usage", error))?;
        let mut usage = BudgetUsage::default();
        for row in rows {
            let (action_json, charged_at) =
                row.map_err(|error| sqlite_error("read budget usage", error))?;
            let action: PlannedAction = decode_json(&action_json)?;
            let asset_lamports = match action.payload {
                ActionPayload::NativeTransfer { lamports, .. }
                | ActionPayload::StakeLifecycle { lamports, .. } => lamports,
                ActionPayload::SplTransfer { .. }
                | ActionPayload::JupiterSwap { .. }
                | ActionPayload::Idle => 0,
            };
            let lamports = asset_lamports
                .checked_add(action.max_fee_lamports)
                .and_then(|value| value.checked_add(action.max_account_creation_lamports))
                .ok_or_else(|| CookerError::Store("action budget usage overflow".to_owned()))?;
            usage.spent_lifetime_lamports = usage
                .spent_lifetime_lamports
                .checked_add(lamports)
                .ok_or_else(|| CookerError::Store("lifetime budget usage overflow".to_owned()))?;
            if parse_timestamp(&charged_at)?.date_naive() == now.date_naive() {
                usage.spent_today_lamports = usage
                    .spent_today_lamports
                    .checked_add(lamports)
                    .ok_or_else(|| CookerError::Store("daily budget usage overflow".to_owned()))?;
            }
        }
        Ok(usage)
    }

    fn release_lease(&self, lease: &ActionLease, at: DateTime<Utc>) -> Result<(), CookerError> {
        self.release_lease_at(lease, at)
    }

    fn defer_action(
        &self,
        lease: &ActionLease,
        until: DateTime<Utc>,
        at: DateTime<Utc>,
        reason: &str,
    ) -> Result<(), CookerError> {
        if until <= at {
            return Err(CookerError::InvalidConfig(
                "action deferral must end after it is recorded".to_owned(),
            ));
        }
        validate_nonempty("deferral reason", reason)?;
        let mut connection = self.lock()?;
        let transaction = immediate(&mut connection, "defer action")?;
        verify_lease(&transaction, lease, at)?;
        if load_action_state(&transaction, &lease.action_id)? != ActionState::Planned {
            return Err(CookerError::Store(format!(
                "only planned action {} can be deferred",
                lease.action_id
            )));
        }
        transaction
            .execute(
                "INSERT INTO action_deferrals(action_id, eligible_at, reason, deferred_at) \
                 VALUES (?1, ?2, ?3, ?4) \
                 ON CONFLICT(action_id) DO UPDATE SET \
                    eligible_at = excluded.eligible_at, \
                    reason = excluded.reason, \
                    deferred_at = excluded.deferred_at",
                params![
                    lease.action_id.as_str(),
                    timestamp(until),
                    reason,
                    timestamp(at)
                ],
            )
            .map_err(|error| sqlite_error("persist action deferral", error))?;
        release_lease_in_transaction(&transaction, lease, at, "deferred")?;
        append_event(
            &transaction,
            &lease.action_id,
            "policy_deferred",
            None,
            None,
            at,
            Some(reason),
            Some(&serde_json::json!({"eligible_at": timestamp(until)})),
        )?;
        transaction
            .commit()
            .map_err(|error| sqlite_error("commit action deferral", error))?;
        Ok(())
    }

    fn rewrite_action(
        &self,
        lease: &ActionLease,
        payload: &ActionPayload,
        at: DateTime<Utc>,
        reason: &str,
    ) -> Result<PlannedAction, CookerError> {
        validate_nonempty("rewrite reason", reason)?;
        let mut connection = self.lock()?;
        let transaction = immediate(&mut connection, "rewrite action")?;
        verify_lease(&transaction, lease, at)?;
        let (original, state) = load_action_and_state(&transaction, &lease.action_id)?;
        if state != ActionState::Planned {
            return Err(CookerError::Store(format!(
                "only planned action {} can be rewritten",
                lease.action_id
            )));
        }
        let has_artifacts: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM prepared_transactions WHERE action_id = ?1) OR \
                        EXISTS(SELECT 1 FROM simulations WHERE action_id = ?1) OR \
                        EXISTS(SELECT 1 FROM submissions WHERE action_id = ?1) OR \
                        EXISTS(SELECT 1 FROM receipts WHERE action_id = ?1)",
                [lease.action_id.as_str()],
                |row| row.get(0),
            )
            .map_err(|error| sqlite_error("inspect action rewrite artifacts", error))?;
        if has_artifacts {
            return Err(CookerError::Store(format!(
                "prepared or observed action {} cannot be rewritten",
                lease.action_id
            )));
        }
        if original.payload == *payload {
            return Err(CookerError::InvalidConfig(
                "policy rewrite must change the action payload".to_owned(),
            ));
        }
        let mut rewritten = original.clone();
        rewritten.payload = payload.clone();
        let rewritten_json = encode_json(&rewritten)?;
        let changed = transaction
            .execute(
                "UPDATE actions SET action_kind = ?2, action_json = ?3, updated_at = ?4 \
                 WHERE action_id = ?1 AND state = 'planned'",
                params![
                    lease.action_id.as_str(),
                    action_kind_name(payload.kind()),
                    rewritten_json,
                    timestamp(at)
                ],
            )
            .map_err(|error| sqlite_error("persist action rewrite", error))?;
        if changed != 1 {
            return Err(CookerError::Store(format!(
                "action {} changed concurrently during rewrite",
                lease.action_id
            )));
        }
        transaction
            .execute(
                "DELETE FROM action_deferrals WHERE action_id = ?1",
                [lease.action_id.as_str()],
            )
            .map_err(|error| sqlite_error("clear rewritten action deferral", error))?;
        append_event(
            &transaction,
            &lease.action_id,
            "policy_rewritten",
            Some(ActionState::Planned),
            Some(ActionState::Planned),
            at,
            Some(reason),
            Some(&serde_json::json!({
                "original_payload": original.payload,
                "replacement_payload": payload,
            })),
        )?;
        transaction
            .commit()
            .map_err(|error| sqlite_error("commit action rewrite", error))?;
        Ok(rewritten)
    }

    fn transition_action(
        &self,
        lease: &ActionLease,
        expected: ActionState,
        next: ActionState,
        at: DateTime<Utc>,
        detail: Option<&str>,
    ) -> Result<(), CookerError> {
        expected.ensure_transition(next)?;
        let mut connection = self.lock()?;
        let transaction = immediate(&mut connection, "transition action")?;
        verify_lease(&transaction, lease, at)?;
        transition_action_in_transaction(&transaction, lease, expected, next, at, detail, false)?;
        transaction
            .commit()
            .map_err(|error| sqlite_error("commit action transition", error))?;
        Ok(())
    }

    fn transition_action_with_trace(
        &self,
        lease: &ActionLease,
        expected: ActionState,
        next: ActionState,
        at: DateTime<Utc>,
        detail: Option<&str>,
        event: &TraceEvent,
    ) -> Result<(), CookerError> {
        expected.ensure_transition(next)?;
        let mut connection = self.lock()?;
        let transaction = immediate(&mut connection, "transition action with trace")?;
        verify_lease(&transaction, lease, at)?;
        transition_action_in_transaction(&transaction, lease, expected, next, at, detail, true)?;
        insert_trace(&transaction, event)?;
        transaction
            .commit()
            .map_err(|error| sqlite_error("commit action transition with trace", error))?;
        Ok(())
    }

    fn record_prepared(
        &self,
        lease: &ActionLease,
        prepared: &PreparedAction,
        at: DateTime<Utc>,
    ) -> Result<(), CookerError> {
        if prepared.action_id != lease.action_id {
            return Err(lease_conflict(
                lease,
                "prepared action does not match leased action",
            ));
        }
        validate_nonempty("prepared signature", &prepared.signature)?;
        validate_nonempty("recent blockhash", &prepared.recent_blockhash)?;
        if prepared.transaction.is_empty() {
            return Err(CookerError::Codec(
                "prepared transaction bytes cannot be empty".to_owned(),
            ));
        }
        let height = to_i64(prepared.last_valid_block_height, "last valid block height")?;
        let expectations = encode_json(&prepared.expectations)?;
        let transaction_hash = blake3::hash(&prepared.transaction).to_hex().to_string();
        let mut connection = self.lock()?;
        let transaction = immediate(&mut connection, "record prepared transaction")?;
        verify_lease(&transaction, lease, at)?;
        if let Some(existing) = load_prepared(&transaction, &lease.action_id)? {
            if existing == *prepared {
                transaction
                    .commit()
                    .map_err(|error| sqlite_error("commit prepared idempotency", error))?;
                return Ok(());
            }
            return Err(CookerError::Store(format!(
                "action {} already has different signed bytes",
                lease.action_id
            )));
        }
        transaction
            .execute(
                "INSERT INTO prepared_transactions(\
                    action_id, signature, transaction_bytes, transaction_hash, recent_blockhash,\
                    last_valid_block_height, expectations_json, prepared_at\
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    lease.action_id.as_str(),
                    prepared.signature,
                    prepared.transaction,
                    transaction_hash,
                    prepared.recent_blockhash,
                    height,
                    expectations,
                    timestamp(at)
                ],
            )
            .map_err(|error| sqlite_error("insert prepared transaction", error))?;
        append_event(
            &transaction,
            &lease.action_id,
            "prepared",
            None,
            None,
            at,
            None,
            Some(&serde_json::json!({
                "signature": prepared.signature,
                "transaction_hash": transaction_hash,
                "recent_blockhash": prepared.recent_blockhash,
                "last_valid_block_height": prepared.last_valid_block_height,
            })),
        )?;
        transaction
            .commit()
            .map_err(|error| sqlite_error("commit prepared transaction", error))?;
        Ok(())
    }

    fn record_simulation(
        &self,
        lease: &ActionLease,
        receipt: &SimulationReceipt,
        at: DateTime<Utc>,
    ) -> Result<(), CookerError> {
        let receipt_json = encode_json(receipt)?;
        let receipt_hash = blake3::hash(receipt_json.as_bytes()).to_hex().to_string();
        let units = receipt
            .units_consumed
            .map(|value| to_i64(value, "simulation units"))
            .transpose()?;
        let mut connection = self.lock()?;
        let transaction = immediate(&mut connection, "record simulation")?;
        verify_lease(&transaction, lease, at)?;
        if load_prepared(&transaction, &lease.action_id)?.is_none() {
            return Err(CookerError::Store(format!(
                "action {} must persist signed bytes before simulation",
                lease.action_id
            )));
        }
        let inserted = transaction
            .execute(
                "INSERT OR IGNORE INTO simulations(\
                    action_id, succeeded, units_consumed, receipt_json, receipt_hash, recorded_at\
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    lease.action_id.as_str(),
                    i64::from(receipt.succeeded),
                    units,
                    receipt_json,
                    receipt_hash,
                    timestamp(at)
                ],
            )
            .map_err(|error| sqlite_error("insert simulation", error))?;
        if inserted == 1 {
            append_event(
                &transaction,
                &lease.action_id,
                "simulation_recorded",
                None,
                None,
                at,
                receipt.error.as_deref(),
                Some(&serde_json::json!({
                    "succeeded": receipt.succeeded,
                    "units_consumed": receipt.units_consumed,
                    "receipt_hash": receipt_hash,
                })),
            )?;
        }
        transaction
            .commit()
            .map_err(|error| sqlite_error("commit simulation", error))?;
        Ok(())
    }

    fn record_submission(
        &self,
        lease: &ActionLease,
        signature: &str,
        at: DateTime<Utc>,
    ) -> Result<(), CookerError> {
        validate_nonempty("submission signature", signature)?;
        let mut connection = self.lock()?;
        let transaction = immediate(&mut connection, "record submission")?;
        verify_lease(&transaction, lease, at)?;
        let prepared = load_prepared(&transaction, &lease.action_id)?.ok_or_else(|| {
            CookerError::Store(format!(
                "action {} cannot be submitted before signed bytes are durable",
                lease.action_id
            ))
        })?;
        if prepared.signature != signature {
            return Err(CookerError::Store(format!(
                "submission signature differs from prepared signature for {}",
                lease.action_id
            )));
        }
        let simulated: bool = transaction
            .query_row(
                "SELECT EXISTS(\
                    SELECT 1 FROM simulations WHERE action_id = ?1 AND succeeded = 1 \
                 )",
                [lease.action_id.as_str()],
                |row| row.get(0),
            )
            .map_err(|error| sqlite_error("verify successful simulation", error))?;
        if !simulated {
            return Err(CookerError::Store(format!(
                "action {} cannot be submitted without a successful simulation",
                lease.action_id
            )));
        }
        if let Some(existing) = load_submission(&transaction, &lease.action_id)? {
            if existing.signature == signature {
                transaction
                    .commit()
                    .map_err(|error| sqlite_error("commit submission idempotency", error))?;
                return Ok(());
            }
            return Err(CookerError::Store(format!(
                "action {} already has a different submission signature",
                lease.action_id
            )));
        }
        transaction
            .execute(
                "INSERT INTO submissions(action_id, signature, submitted_at) \
                 VALUES (?1, ?2, ?3)",
                params![lease.action_id.as_str(), signature, timestamp(at)],
            )
            .map_err(|error| sqlite_error("insert submission", error))?;
        append_event(
            &transaction,
            &lease.action_id,
            "submission_recorded",
            None,
            None,
            at,
            None,
            Some(&serde_json::json!({"signature": signature})),
        )?;
        transaction
            .commit()
            .map_err(|error| sqlite_error("commit submission", error))?;
        Ok(())
    }

    fn record_receipt(
        &self,
        lease: &ActionLease,
        receipt: &ChainReceipt,
    ) -> Result<(), CookerError> {
        let mut connection = self.lock()?;
        let transaction = immediate(&mut connection, "record chain receipt")?;
        verify_lease(&transaction, lease, receipt.observed_at)?;
        insert_receipt(&transaction, lease, receipt)?;
        transaction
            .commit()
            .map_err(|error| sqlite_error("commit chain receipt", error))?;
        Ok(())
    }

    fn claim_reconciliation_candidates(
        &self,
        worker_id: &str,
        now: DateTime<Utc>,
        lease_until: DateTime<Utc>,
        limit: usize,
    ) -> Result<Vec<ActionLease>, CookerError> {
        self.claim(worker_id, now, lease_until, limit, true)
    }

    fn claim_confirmation_audit_candidates(
        &self,
        worker_id: &str,
        now: DateTime<Utc>,
        lease_until: DateTime<Utc>,
        due_before: DateTime<Utc>,
        limit: usize,
    ) -> Result<Vec<ActionLease>, CookerError> {
        self.claim_confirmation_audits(worker_id, now, lease_until, due_before, limit)
    }

    fn complete_confirmation_audit(
        &self,
        lease: &ActionLease,
        receipt: &ChainReceipt,
        audited_at: DateTime<Utc>,
    ) -> Result<(), CookerError> {
        validate_successful_audit(receipt)?;
        let mut connection = self.lock()?;
        let transaction = immediate(&mut connection, "complete confirmation audit")?;
        verify_lease(&transaction, lease, audited_at)?;
        ensure_action_state(&transaction, &lease.action_id, ActionState::Confirmed)?;
        let receipt_hash = insert_receipt(&transaction, lease, receipt)?;
        insert_confirmation_audit(
            &transaction,
            lease,
            receipt,
            &receipt_hash,
            audited_at,
            ConfirmationAuditOutcome::Verified,
        )?;
        append_event(
            &transaction,
            &lease.action_id,
            "confirmation_audit_verified",
            None,
            None,
            audited_at,
            None,
            Some(&audit_event_payload(receipt, &receipt_hash)),
        )?;
        release_lease_in_transaction(&transaction, lease, audited_at, "confirmation_audited")?;
        transaction
            .commit()
            .map_err(|error| sqlite_error("commit confirmation audit", error))?;
        Ok(())
    }

    fn orphan_confirmation(
        &self,
        lease: &ActionLease,
        receipt: &ChainReceipt,
        audited_at: DateTime<Utc>,
        reason: &str,
        event: &TraceEvent,
    ) -> Result<(), CookerError> {
        validate_orphaned_audit(receipt)?;
        validate_nonempty("confirmation orphan reason", reason)?;
        ActionState::Confirmed.ensure_transition(ActionState::Orphaned)?;
        let mut connection = self.lock()?;
        let transaction = immediate(&mut connection, "orphan confirmation")?;
        verify_lease(&transaction, lease, audited_at)?;
        let receipt_hash = insert_receipt(&transaction, lease, receipt)?;
        insert_confirmation_audit(
            &transaction,
            lease,
            receipt,
            &receipt_hash,
            audited_at,
            ConfirmationAuditOutcome::Orphaned,
        )?;
        transition_action_in_transaction(
            &transaction,
            lease,
            ActionState::Confirmed,
            ActionState::Orphaned,
            audited_at,
            Some(reason),
            true,
        )?;
        insert_trace(&transaction, event)?;
        append_event(
            &transaction,
            &lease.action_id,
            "confirmation_audit_orphaned",
            Some(ActionState::Confirmed),
            Some(ActionState::Orphaned),
            audited_at,
            Some(reason),
            Some(&audit_event_payload(receipt, &receipt_hash)),
        )?;
        release_lease_in_transaction(&transaction, lease, audited_at, "confirmation_orphaned")?;
        transaction
            .commit()
            .map_err(|error| sqlite_error("commit orphaned confirmation", error))?;
        Ok(())
    }

    fn append_trace(&self, event: &TraceEvent) -> Result<(), CookerError> {
        let mut connection = self.lock()?;
        let transaction = immediate(&mut connection, "append trace")?;
        insert_trace(&transaction, event)?;
        transaction
            .commit()
            .map_err(|error| sqlite_error("commit trace", error))?;
        Ok(())
    }
}

const ACTION_STATES: [ActionState; 10] = [
    ActionState::Planned,
    ActionState::Simulated,
    ActionState::Submitted,
    ActionState::Confirmed,
    ActionState::Rejected,
    ActionState::Failed,
    ActionState::Expired,
    ActionState::Unknown,
    ActionState::Orphaned,
    ActionState::Cancelled,
];

fn validate_audit_cutoff(
    now: DateTime<Utc>,
    confirmation_audit_due_before: Option<DateTime<Utc>>,
) -> Result<(), CookerError> {
    if confirmation_audit_due_before.is_some_and(|cutoff| cutoff > now) {
        Err(CookerError::InvalidConfig(
            "confirmation audit cutoff cannot be in the future".to_owned(),
        ))
    } else {
        Ok(())
    }
}

fn query_count<P: rusqlite::Params>(
    connection: &Connection,
    query: &str,
    parameters: P,
    label: &str,
) -> Result<u64, CookerError> {
    let count: i64 = connection
        .query_row(query, parameters, |row| row.get(0))
        .map_err(|error| sqlite_error(&format!("query {label} count"), error))?;
    from_i64(count, label)
}

fn load_action_state_counts(
    connection: &Connection,
) -> Result<std::collections::BTreeMap<ActionState, u64>, CookerError> {
    let mut counts = ACTION_STATES
        .into_iter()
        .map(|state| (state, 0))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut statement = connection
        .prepare("SELECT state, count(*) FROM actions GROUP BY state ORDER BY state")
        .map_err(|error| sqlite_error("prepare action state counts", error))?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|error| sqlite_error("query action state counts", error))?;
    for row in rows {
        let (state, count) = row.map_err(|error| sqlite_error("read action state count", error))?;
        counts.insert(parse_state(&state)?, from_i64(count, "action state count")?);
    }
    Ok(counts)
}

fn count_expired_leases(connection: &Connection, now: DateTime<Utc>) -> Result<u64, CookerError> {
    query_count(
        connection,
        "SELECT count(*) FROM leases \
         WHERE released_at IS NULL AND expires_at <= ?1",
        [timestamp(now)],
        "expired leases",
    )
}

fn count_due_actions(connection: &Connection, now: DateTime<Utc>) -> Result<u64, CookerError> {
    query_count(
        connection,
        "SELECT count(*) FROM actions a \
         LEFT JOIN action_deferrals d ON d.action_id = a.action_id \
         WHERE COALESCE(d.eligible_at, a.scheduled_at) <= ?1 \
           AND a.state IN ('planned', 'simulated') \
           AND NOT EXISTS ( \
               SELECT 1 FROM submissions s WHERE s.action_id = a.action_id \
           ) \
           AND NOT EXISTS ( \
               SELECT 1 FROM leases l \
               WHERE l.action_id = a.action_id AND l.released_at IS NULL \
                 AND l.expires_at > ?1 \
           )",
        [timestamp(now)],
        "due actions",
    )
}

fn count_reconciliation_candidates(
    connection: &Connection,
    now: DateTime<Utc>,
) -> Result<u64, CookerError> {
    query_count(
        connection,
        "SELECT count(*) FROM actions a \
         WHERE ( \
             a.state IN ('submitted', 'unknown', 'orphaned') \
             OR (a.state = 'simulated' AND EXISTS ( \
                 SELECT 1 FROM submissions s WHERE s.action_id = a.action_id \
             )) \
         ) \
         AND NOT EXISTS ( \
             SELECT 1 FROM leases l \
             WHERE l.action_id = a.action_id AND l.released_at IS NULL \
               AND l.expires_at > ?1 \
         )",
        [timestamp(now)],
        "reconciliation candidates",
    )
}

fn count_due_confirmation_audits(
    connection: &Connection,
    now: DateTime<Utc>,
    due_before: DateTime<Utc>,
) -> Result<u64, CookerError> {
    query_count(
        connection,
        "SELECT count(*) FROM actions a \
         WHERE a.state = 'confirmed' \
           AND COALESCE(( \
               SELECT MAX(ca.audited_at) FROM confirmation_audits ca \
               WHERE ca.action_id = a.action_id \
           ), a.updated_at) < ?1 \
           AND NOT EXISTS ( \
               SELECT 1 FROM leases l \
               WHERE l.action_id = a.action_id AND l.released_at IS NULL \
                 AND l.expires_at > ?2 \
           )",
        params![timestamp(due_before), timestamp(now)],
        "confirmation audit candidates",
    )
}

fn load_expired_lease_candidates(
    connection: &Connection,
    now: DateTime<Utc>,
    limit: i64,
) -> Result<Vec<ExpiredLeaseCandidate>, CookerError> {
    let mut statement = connection
        .prepare(
            "SELECT l.lease_id, l.action_id, l.worker_id, l.acquired_at, l.expires_at, a.state \
             FROM leases l JOIN actions a ON a.action_id = l.action_id \
             WHERE l.released_at IS NULL AND l.expires_at <= ?1 \
             ORDER BY l.expires_at, l.lease_id LIMIT ?2",
        )
        .map_err(|error| sqlite_error("prepare expired lease preview", error))?;
    let rows = statement
        .query_map(params![timestamp(now), limit], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .map_err(|error| sqlite_error("query expired lease preview", error))?;
    let mut candidates = Vec::new();
    for row in rows {
        let (lease_id, action_id, worker_id, acquired_at, expires_at, state) =
            row.map_err(|error| sqlite_error("read expired lease preview", error))?;
        candidates.push(ExpiredLeaseCandidate {
            lease: ActionLease {
                id: LeaseId(parse_uuid(&lease_id, "lease id")?),
                action_id: decode_action_id(&action_id)?,
                worker_id,
                expires_at: parse_timestamp(&expires_at)?,
            },
            acquired_at: parse_timestamp(&acquired_at)?,
            action_state: parse_state(&state)?,
        });
    }
    Ok(candidates)
}

fn load_reconciliation_candidates(
    connection: &Connection,
    now: DateTime<Utc>,
    limit: i64,
) -> Result<Vec<RecoveryActionCandidate>, CookerError> {
    let mut statement = connection
        .prepare(
            "SELECT a.action_id, a.run_id, a.agent_id, a.state, s.signature, a.updated_at \
             FROM actions a LEFT JOIN submissions s ON s.action_id = a.action_id \
             WHERE ( \
                 a.state IN ('submitted', 'unknown', 'orphaned') \
                 OR (a.state = 'simulated' AND s.action_id IS NOT NULL) \
             ) \
             AND NOT EXISTS ( \
                 SELECT 1 FROM leases l \
                 WHERE l.action_id = a.action_id AND l.released_at IS NULL \
                   AND l.expires_at > ?1 \
             ) \
             ORDER BY a.updated_at, a.action_id LIMIT ?2",
        )
        .map_err(|error| sqlite_error("prepare reconciliation preview", error))?;
    let rows = statement
        .query_map(params![timestamp(now), limit], recovery_candidate_row)
        .map_err(|error| sqlite_error("query reconciliation preview", error))?;
    decode_recovery_candidates(rows, "reconciliation preview")
}

fn load_confirmation_audit_candidates(
    connection: &Connection,
    now: DateTime<Utc>,
    due_before: DateTime<Utc>,
    limit: i64,
) -> Result<Vec<RecoveryActionCandidate>, CookerError> {
    let mut statement = connection
        .prepare(
            "SELECT a.action_id, a.run_id, a.agent_id, a.state, s.signature, \
                    COALESCE(( \
                        SELECT MAX(ca.audited_at) FROM confirmation_audits ca \
                        WHERE ca.action_id = a.action_id \
                    ), a.updated_at) \
             FROM actions a LEFT JOIN submissions s ON s.action_id = a.action_id \
             WHERE a.state = 'confirmed' \
               AND COALESCE(( \
                   SELECT MAX(ca.audited_at) FROM confirmation_audits ca \
                   WHERE ca.action_id = a.action_id \
               ), a.updated_at) < ?1 \
               AND NOT EXISTS ( \
                   SELECT 1 FROM leases l \
                   WHERE l.action_id = a.action_id AND l.released_at IS NULL \
                     AND l.expires_at > ?2 \
               ) \
             ORDER BY 6, a.action_id LIMIT ?3",
        )
        .map_err(|error| sqlite_error("prepare confirmation audit preview", error))?;
    let rows = statement
        .query_map(
            params![timestamp(due_before), timestamp(now), limit],
            recovery_candidate_row,
        )
        .map_err(|error| sqlite_error("query confirmation audit preview", error))?;
    decode_recovery_candidates(rows, "confirmation audit preview")
}

type RecoveryCandidateSqlRow = (String, String, String, String, Option<String>, String);

fn recovery_candidate_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RecoveryCandidateSqlRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
    ))
}

fn decode_recovery_candidates(
    rows: rusqlite::MappedRows<
        '_,
        impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<RecoveryCandidateSqlRow>,
    >,
    label: &str,
) -> Result<Vec<RecoveryActionCandidate>, CookerError> {
    let mut candidates = Vec::new();
    for row in rows {
        let (action_id, run_id, agent_id, state, signature, eligible_since) =
            row.map_err(|error| sqlite_error(&format!("read {label}"), error))?;
        candidates.push(RecoveryActionCandidate {
            action_id: decode_action_id(&action_id)?,
            run_id: RunId(parse_uuid(&run_id, "run id")?),
            agent_id: AgentId(parse_uuid(&agent_id, "agent id")?),
            state: parse_state(&state)?,
            signature,
            eligible_since: parse_timestamp(&eligible_since)?,
        });
    }
    Ok(candidates)
}

fn immediate<'connection>(
    connection: &'connection mut Connection,
    context: &str,
) -> Result<Transaction<'connection>, CookerError> {
    connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| sqlite_error(context, error))
}

fn release_lease_in_transaction(
    transaction: &Transaction<'_>,
    lease: &ActionLease,
    at: DateTime<Utc>,
    reason: &str,
) -> Result<(), CookerError> {
    let changed = transaction
        .execute(
            "UPDATE leases SET released_at = ?2, release_reason = ?3 \
             WHERE lease_id = ?1 AND released_at IS NULL",
            params![lease.id.to_string(), timestamp(at), reason],
        )
        .map_err(|error| sqlite_error("release lease", error))?;
    if changed != 1 {
        return Err(lease_conflict(lease, "lease was concurrently released"));
    }
    Ok(())
}

fn transition_action_in_transaction(
    transaction: &Transaction<'_>,
    lease: &ActionLease,
    expected: ActionState,
    next: ActionState,
    at: DateTime<Utc>,
    detail: Option<&str>,
    has_atomic_trace: bool,
) -> Result<(), CookerError> {
    let actual = load_action_state(transaction, &lease.action_id)?;
    if actual != expected {
        return Err(CookerError::Store(format!(
            "action {} state compare-and-swap expected {}, found {}",
            lease.action_id,
            state_name(expected),
            state_name(actual)
        )));
    }
    verify_transition_artifacts(
        transaction,
        &lease.action_id,
        expected,
        next,
        has_atomic_trace,
    )?;
    let changed = transaction
        .execute(
            "UPDATE actions SET state = ?3, updated_at = ?4 \
             WHERE action_id = ?1 AND state = ?2",
            params![
                lease.action_id.as_str(),
                state_name(expected),
                state_name(next),
                timestamp(at)
            ],
        )
        .map_err(|error| sqlite_error("compare-and-swap action state", error))?;
    if changed != 1 {
        return Err(CookerError::Store(format!(
            "action {} state changed concurrently",
            lease.action_id
        )));
    }
    append_event(
        transaction,
        &lease.action_id,
        "state_transition",
        Some(expected),
        Some(next),
        at,
        detail,
        None,
    )
}

fn verify_transition_artifacts(
    transaction: &Transaction<'_>,
    action_id: &ActionId,
    expected: ActionState,
    next: ActionState,
    has_atomic_trace: bool,
) -> Result<(), CookerError> {
    if next.is_terminal() && !has_atomic_trace {
        return Err(CookerError::Store(format!(
            "terminal action {action_id} transition to {} requires an atomic trace",
            state_name(next)
        )));
    }
    let required = match next {
        ActionState::Simulated => Some((
            "SELECT EXISTS(SELECT 1 FROM prepared_transactions WHERE action_id = ?1) AND \
             EXISTS(SELECT 1 FROM simulations WHERE action_id = ?1 AND succeeded = 1)",
            "durable signed bytes and a successful simulation",
        )),
        ActionState::Submitted | ActionState::Unknown => Some((
            "SELECT EXISTS(SELECT 1 FROM submissions WHERE action_id = ?1)",
            "a durable submission signature",
        )),
        ActionState::Confirmed => Some((
            "SELECT EXISTS(SELECT 1 FROM receipts WHERE action_id = ?1 AND \
             confirmation_status IN ('confirmed', 'finalized') AND postconditions_met = 1)",
            "a confirmed receipt with proven postconditions",
        )),
        ActionState::Failed => Some((
            "SELECT EXISTS(SELECT 1 FROM receipts WHERE action_id = ?1 AND \
             confirmation_status = 'failed')",
            "a failed chain receipt",
        )),
        ActionState::Orphaned if expected == ActionState::Confirmed => Some((
            "SELECT EXISTS(SELECT 1 FROM confirmation_audits WHERE action_id = ?1 AND \
             outcome = 'orphaned')",
            "an immutable orphaning confirmation audit",
        )),
        ActionState::Planned
        | ActionState::Rejected
        | ActionState::Expired
        | ActionState::Cancelled
        | ActionState::Orphaned => None,
    };
    if let Some((query, evidence)) = required {
        let exists: bool = transaction
            .query_row(query, [action_id.as_str()], |row| row.get(0))
            .map_err(|error| sqlite_error("verify action transition artifacts", error))?;
        if !exists {
            return Err(CookerError::Store(format!(
                "action {action_id} cannot transition to {} without {evidence}",
                state_name(next)
            )));
        }
    }
    Ok(())
}

fn ensure_action_state(
    connection: &Connection,
    action_id: &ActionId,
    expected: ActionState,
) -> Result<(), CookerError> {
    let actual = load_action_state(connection, action_id)?;
    if actual == expected {
        Ok(())
    } else {
        Err(CookerError::Store(format!(
            "action {action_id} state compare-and-swap expected {}, found {}",
            state_name(expected),
            state_name(actual)
        )))
    }
}

fn insert_receipt(
    transaction: &Transaction<'_>,
    lease: &ActionLease,
    receipt: &ChainReceipt,
) -> Result<String, CookerError> {
    if receipt.action_id != lease.action_id {
        return Err(lease_conflict(
            lease,
            "receipt action does not match leased action",
        ));
    }
    let submission = load_submission(transaction, &lease.action_id)?.ok_or_else(|| {
        CookerError::Store(format!(
            "action {} has no durable submission to observe",
            lease.action_id
        ))
    })?;
    if submission.signature != receipt.signature {
        return Err(CookerError::Store(format!(
            "receipt signature differs from durable submission for {}",
            lease.action_id
        )));
    }
    let receipt_json = encode_json(receipt)?;
    let receipt_hash = blake3::hash(receipt_json.as_bytes()).to_hex().to_string();
    let slot = receipt
        .slot
        .map(|value| to_i64(value, "receipt slot"))
        .transpose()?;
    let inserted = transaction
        .execute(
            "INSERT OR IGNORE INTO receipts( \
                action_id, signature, confirmation_status, slot, postconditions_met, \
                receipt_json, receipt_hash, observed_at \
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                lease.action_id.as_str(),
                receipt.signature,
                confirmation_status_name(receipt.status),
                slot,
                i64::from(receipt.postconditions_met),
                receipt_json,
                receipt_hash,
                timestamp(receipt.observed_at)
            ],
        )
        .map_err(|error| sqlite_error("insert chain receipt", error))?;
    if inserted == 1 {
        append_event(
            transaction,
            &lease.action_id,
            "receipt_recorded",
            None,
            None,
            receipt.observed_at,
            receipt.error.as_deref(),
            Some(&serde_json::json!({
                "signature": receipt.signature,
                "status": confirmation_status_name(receipt.status),
                "slot": receipt.slot,
                "postconditions_met": receipt.postconditions_met,
                "receipt_hash": receipt_hash,
            })),
        )?;
    }
    Ok(receipt_hash)
}

fn insert_confirmation_audit(
    transaction: &Transaction<'_>,
    lease: &ActionLease,
    receipt: &ChainReceipt,
    receipt_hash: &str,
    audited_at: DateTime<Utc>,
    outcome: ConfirmationAuditOutcome,
) -> Result<(), CookerError> {
    let slot = receipt
        .slot
        .map(|value| to_i64(value, "confirmation audit slot"))
        .transpose()?;
    transaction
        .execute(
            "INSERT INTO confirmation_audits( \
                action_id, signature, outcome, confirmation_status, slot, postconditions_met, \
                receipt_json, receipt_hash, audited_at \
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                lease.action_id.as_str(),
                receipt.signature,
                audit_outcome_name(outcome),
                confirmation_status_name(receipt.status),
                slot,
                i64::from(receipt.postconditions_met),
                encode_json(receipt)?,
                receipt_hash,
                timestamp(audited_at)
            ],
        )
        .map_err(|error| sqlite_error("insert confirmation audit", error))?;
    Ok(())
}

fn audit_event_payload(receipt: &ChainReceipt, receipt_hash: &str) -> Value {
    serde_json::json!({
        "signature": receipt.signature,
        "status": confirmation_status_name(receipt.status),
        "slot": receipt.slot,
        "postconditions_met": receipt.postconditions_met,
        "receipt_hash": receipt_hash,
    })
}

fn validate_successful_audit(receipt: &ChainReceipt) -> Result<(), CookerError> {
    if matches!(
        receipt.status,
        ConfirmationStatus::Confirmed | ConfirmationStatus::Finalized
    ) && receipt.postconditions_met
    {
        Ok(())
    } else {
        Err(CookerError::Store(
            "successful confirmation audit requires confirmed state and proven postconditions"
                .to_owned(),
        ))
    }
}

fn validate_orphaned_audit(receipt: &ChainReceipt) -> Result<(), CookerError> {
    if matches!(
        receipt.status,
        ConfirmationStatus::Confirmed | ConfirmationStatus::Finalized
    ) && receipt.postconditions_met
    {
        Err(CookerError::Store(
            "a still-confirmed receipt cannot orphan an action".to_owned(),
        ))
    } else {
        Ok(())
    }
}

fn insert_trace(transaction: &Transaction<'_>, event: &TraceEvent) -> Result<(), CookerError> {
    if event.schema_version != TraceEvent::SCHEMA_VERSION {
        return Err(CookerError::Codec(format!(
            "unsupported trace schema version {}",
            event.schema_version
        )));
    }
    let action = load_action_and_state(transaction, &event.action_id)?.0;
    if action.run_id != event.run_id
        || action.agent_id != event.agent_id
        || action.sequence != event.sequence
        || action.payload.kind() != event.action_kind
        || action.scheduled_at != event.scheduled_at
    {
        return Err(CookerError::Store(format!(
            "trace metadata does not match action {}",
            event.action_id
        )));
    }
    let event_json = encode_json(event)?;
    let event_hash = blake3::hash(event_json.as_bytes()).to_hex().to_string();
    transaction
        .execute(
            "INSERT OR IGNORE INTO traces(\
                schema_version, run_id, agent_id, action_id, event_json, event_hash, appended_at\
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                i64::from(event.schema_version),
                event.run_id.to_string(),
                event.agent_id.to_string(),
                event.action_id.as_str(),
                event_json,
                event_hash,
                timestamp(event.observed_at.unwrap_or(event.scheduled_at))
            ],
        )
        .map_err(|error| sqlite_error("insert trace", error))?;
    Ok(())
}

fn ensure_run_stub(
    transaction: &Transaction<'_>,
    run_id: RunId,
    at: DateTime<Utc>,
) -> Result<(), CookerError> {
    transaction
        .execute(
            "INSERT OR IGNORE INTO runs(run_id, status, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?3)",
            params![run_id.to_string(), ACTIVE_RUN, timestamp(at)],
        )
        .map_err(|error| sqlite_error("ensure run stub", error))?;
    Ok(())
}

fn ensure_agent_stub(
    transaction: &Transaction<'_>,
    action: &PlannedAction,
) -> Result<(), CookerError> {
    let existing_run: Option<String> = transaction
        .query_row(
            "SELECT run_id FROM agents WHERE agent_id = ?1",
            [action.agent_id.to_string()],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| sqlite_error("load action agent ownership", error))?;
    if existing_run
        .as_deref()
        .is_some_and(|run| run != action.run_id.to_string())
    {
        return Err(CookerError::Store(format!(
            "agent {} belongs to a different run",
            action.agent_id
        )));
    }
    if existing_run.is_some() {
        return Ok(());
    }
    let snapshot = AgentSnapshot {
        id: action.agent_id,
        run_id: action.run_id,
        next_sequence: action.sequence.saturating_add(1),
        next_decision_at: action.scheduled_at,
        budget_date: action.scheduled_at.date_naive(),
        session_state: SessionState::Dormant,
        last_action_at: None,
        remaining_daily_budget: 0,
        model_version: action.model_version.clone(),
    };
    transaction
        .execute(
            "INSERT INTO agents(\
                agent_id, run_id, next_sequence, next_decision_at, budget_date, session_state,\
                remaining_daily_budget, model_version, snapshot_json, created_at, updated_at\
             ) VALUES (?1, ?2, ?3, ?4, ?5, 'dormant', 0, ?6, ?7, ?8, ?8)",
            params![
                action.agent_id.to_string(),
                action.run_id.to_string(),
                to_i64(snapshot.next_sequence, "agent sequence stub")?,
                timestamp(snapshot.next_decision_at),
                snapshot.budget_date.to_string(),
                action.model_version,
                encode_json(&snapshot)?,
                timestamp(action.created_at)
            ],
        )
        .map_err(|error| sqlite_error("insert agent stub", error))?;
    Ok(())
}

fn expire_leases(transaction: &Transaction<'_>, now: DateTime<Utc>) -> Result<usize, CookerError> {
    let expired = {
        let mut statement = transaction
            .prepare(
                "SELECT lease_id, action_id, worker_id FROM leases \
                 WHERE released_at IS NULL AND expires_at <= ?1 \
                 ORDER BY expires_at, lease_id",
            )
            .map_err(|error| sqlite_error("prepare expired lease query", error))?;
        let rows = statement
            .query_map([timestamp(now)], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|error| sqlite_error("query expired leases", error))?;
        let mut leases = Vec::new();
        for row in rows {
            leases.push(row.map_err(|error| sqlite_error("read expired lease", error))?);
        }
        leases
    };
    for (lease_id, action_id_text, worker_id) in &expired {
        transaction
            .execute(
                "UPDATE leases SET released_at = ?2, release_reason = 'expired' \
                 WHERE lease_id = ?1 AND released_at IS NULL",
                params![lease_id, timestamp(now)],
            )
            .map_err(|error| sqlite_error("expire lease", error))?;
        let action_id = decode_action_id(action_id_text)?;
        append_event(
            transaction,
            &action_id,
            "lease_expired",
            None,
            None,
            now,
            Some(worker_id),
            Some(&serde_json::json!({"lease_id": lease_id})),
        )?;
    }
    Ok(expired.len())
}

fn verify_lease(
    connection: &Connection,
    lease: &ActionLease,
    at: DateTime<Utc>,
) -> Result<(), CookerError> {
    let stored: Option<(String, String, String, Option<String>)> = connection
        .query_row(
            "SELECT action_id, worker_id, expires_at, released_at \
             FROM leases WHERE lease_id = ?1",
            [lease.id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(|error| sqlite_error("verify lease", error))?;
    let Some((action_id, worker_id, expires_at, released_at)) = stored else {
        return Err(lease_conflict(lease, "lease id is unknown"));
    };
    let stored_expiry = parse_timestamp(&expires_at)?;
    if action_id != lease.action_id.as_str() {
        return Err(lease_conflict(lease, "lease action id does not match"));
    }
    if worker_id != lease.worker_id {
        return Err(lease_conflict(lease, "lease worker id does not match"));
    }
    if stored_expiry != lease.expires_at {
        return Err(lease_conflict(lease, "lease expiry does not match"));
    }
    if released_at.is_some() {
        return Err(lease_conflict(lease, "lease was released"));
    }
    if at >= stored_expiry {
        return Err(lease_conflict(lease, "lease expired"));
    }
    Ok(())
}

fn load_action_and_state(
    connection: &Connection,
    action_id: &ActionId,
) -> Result<(PlannedAction, ActionState), CookerError> {
    connection
        .query_row(
            "SELECT action_json, state FROM actions WHERE action_id = ?1",
            [action_id.as_str()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|error| sqlite_error("load action", error))?
        .map_or_else(
            || Err(CookerError::NotFound(format!("action {action_id}"))),
            |(json, state)| Ok((decode_json(&json)?, parse_state(&state)?)),
        )
}

fn load_action_state(
    connection: &Connection,
    action_id: &ActionId,
) -> Result<ActionState, CookerError> {
    load_action_and_state(connection, action_id).map(|(_, state)| state)
}

fn load_prepared(
    connection: &Connection,
    action_id: &ActionId,
) -> Result<Option<PreparedAction>, CookerError> {
    let row: Option<(String, Vec<u8>, String, i64, String)> = connection
        .query_row(
            "SELECT signature, transaction_bytes, recent_blockhash, \
                    last_valid_block_height, expectations_json \
             FROM prepared_transactions WHERE action_id = ?1",
            [action_id.as_str()],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()
        .map_err(|error| sqlite_error("load prepared transaction", error))?;
    row.map(
        |(signature, transaction, recent_blockhash, last_valid_height, expectations)| {
            Ok(PreparedAction {
                action_id: action_id.clone(),
                signature,
                transaction,
                recent_blockhash,
                last_valid_block_height: from_i64(last_valid_height, "last valid block height")?,
                expectations: decode_json::<Vec<StateExpectation>>(&expectations)?,
            })
        },
    )
    .transpose()
}

fn load_simulations(
    connection: &Connection,
    action_id: &ActionId,
) -> Result<Vec<SimulationRecord>, CookerError> {
    let mut statement = connection
        .prepare(
            "SELECT receipt_json, recorded_at FROM simulations \
             WHERE action_id = ?1 ORDER BY simulation_id",
        )
        .map_err(|error| sqlite_error("prepare simulation query", error))?;
    let rows = statement
        .query_map([action_id.as_str()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| sqlite_error("query simulations", error))?;
    let mut simulations = Vec::new();
    for row in rows {
        let (receipt, at) = row.map_err(|error| sqlite_error("read simulation", error))?;
        simulations.push(SimulationRecord {
            receipt: decode_json(&receipt)?,
            recorded_at: parse_timestamp(&at)?,
        });
    }
    Ok(simulations)
}

fn load_submission(
    connection: &Connection,
    action_id: &ActionId,
) -> Result<Option<SubmissionRecord>, CookerError> {
    connection
        .query_row(
            "SELECT signature, submitted_at FROM submissions WHERE action_id = ?1",
            [action_id.as_str()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|error| sqlite_error("load submission", error))?
        .map(|(signature, submitted_at)| {
            Ok(SubmissionRecord {
                signature,
                submitted_at: parse_timestamp(&submitted_at)?,
            })
        })
        .transpose()
}

fn load_receipts(
    connection: &Connection,
    action_id: &ActionId,
) -> Result<Vec<ChainReceipt>, CookerError> {
    let mut statement = connection
        .prepare("SELECT receipt_json FROM receipts WHERE action_id = ?1 ORDER BY receipt_id")
        .map_err(|error| sqlite_error("prepare receipt query", error))?;
    let rows = statement
        .query_map([action_id.as_str()], |row| row.get::<_, String>(0))
        .map_err(|error| sqlite_error("query receipts", error))?;
    let mut receipts = Vec::new();
    for row in rows {
        receipts.push(decode_json(
            &row.map_err(|error| sqlite_error("read receipt", error))?,
        )?);
    }
    Ok(receipts)
}

fn load_active_lease(
    connection: &Connection,
    action_id: &ActionId,
    now: DateTime<Utc>,
) -> Result<Option<ActionLease>, CookerError> {
    let row: Option<(String, String, String)> = connection
        .query_row(
            "SELECT lease_id, worker_id, expires_at FROM leases \
             WHERE action_id = ?1 AND released_at IS NULL AND expires_at > ?2",
            params![action_id.as_str(), timestamp(now)],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|error| sqlite_error("load active lease", error))?;
    row.map(|(lease_id, worker_id, expires_at)| {
        Ok(ActionLease {
            id: LeaseId(parse_uuid(&lease_id, "lease id")?),
            action_id: action_id.clone(),
            worker_id,
            expires_at: parse_timestamp(&expires_at)?,
        })
    })
    .transpose()
}

fn load_events(
    connection: &Connection,
    action_id: &ActionId,
) -> Result<Vec<ActionEventRecord>, CookerError> {
    let mut statement = connection
        .prepare(
            "SELECT event_id, event_kind, from_state, to_state, occurred_at, detail, payload_json \
             FROM action_events WHERE action_id = ?1 ORDER BY event_id",
        )
        .map_err(|error| sqlite_error("prepare event query", error))?;
    let rows = statement
        .query_map([action_id.as_str()], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
            ))
        })
        .map_err(|error| sqlite_error("query action events", error))?;
    let mut events = Vec::new();
    for row in rows {
        let (id, kind, from, to, at, detail, payload) =
            row.map_err(|error| sqlite_error("read action event", error))?;
        events.push(ActionEventRecord {
            id,
            kind,
            from_state: from.map(|value| parse_state(&value)).transpose()?,
            to_state: to.map(|value| parse_state(&value)).transpose()?,
            occurred_at: parse_timestamp(&at)?,
            detail,
            payload: payload.map(|json| decode_json(&json)).transpose()?,
        });
    }
    Ok(events)
}

#[allow(clippy::too_many_arguments)]
fn append_event(
    transaction: &Transaction<'_>,
    action_id: &ActionId,
    kind: &str,
    from_state: Option<ActionState>,
    to_state: Option<ActionState>,
    at: DateTime<Utc>,
    detail: Option<&str>,
    payload: Option<&Value>,
) -> Result<(), CookerError> {
    transaction
        .execute(
            "INSERT INTO action_events(\
                action_id, event_kind, from_state, to_state, occurred_at, detail, payload_json\
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                action_id.as_str(),
                kind,
                from_state.map(state_name),
                to_state.map(state_name),
                timestamp(at),
                detail,
                payload.map(encode_json).transpose()?
            ],
        )
        .map_err(|error| sqlite_error("append immutable action event", error))?;
    Ok(())
}

fn validate_claim(
    worker_id: &str,
    now: DateTime<Utc>,
    lease_until: DateTime<Utc>,
    limit: usize,
) -> Result<(), CookerError> {
    validate_nonempty("worker_id", worker_id)?;
    if lease_until <= now {
        return Err(CookerError::InvalidConfig(
            "lease expiry must be later than claim time".to_owned(),
        ));
    }
    if limit == 0 {
        return Err(CookerError::InvalidConfig(
            "claim limit must be positive".to_owned(),
        ));
    }
    Ok(())
}

fn validate_action(action: &PlannedAction) -> Result<(), CookerError> {
    validate_nonempty("model_version", &action.model_version)?;
    let derived = ActionId::derive(
        action.run_id,
        action.agent_id,
        action.sequence,
        &action.model_version,
    );
    if derived != action.id {
        return Err(CookerError::Store(format!(
            "action id {} does not match its deterministic inputs",
            action.id
        )));
    }
    Ok(())
}

fn validate_nonempty(field: &str, value: &str) -> Result<(), CookerError> {
    if value.trim().is_empty() {
        return Err(CookerError::InvalidConfig(format!(
            "{field} cannot be empty"
        )));
    }
    Ok(())
}

fn validate_hash(field: &str, value: &str) -> Result<(), CookerError> {
    validate_nonempty(field, value)?;
    if !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(CookerError::InvalidConfig(format!(
            "{field} must be hexadecimal"
        )));
    }
    Ok(())
}

fn validate_run_status(status: &str) -> Result<(), CookerError> {
    if matches!(status, "active" | "completed" | "cancelled" | "failed") {
        Ok(())
    } else {
        Err(CookerError::InvalidConfig(format!(
            "unsupported run status {status}"
        )))
    }
}

fn lease_conflict(lease: &ActionLease, reason: &str) -> CookerError {
    CookerError::LeaseConflict(format!(
        "lease {} for action {}: {reason}",
        lease.id, lease.action_id
    ))
}

fn state_name(state: ActionState) -> &'static str {
    match state {
        ActionState::Planned => "planned",
        ActionState::Simulated => "simulated",
        ActionState::Submitted => "submitted",
        ActionState::Confirmed => "confirmed",
        ActionState::Rejected => "rejected",
        ActionState::Failed => "failed",
        ActionState::Expired => "expired",
        ActionState::Unknown => "unknown",
        ActionState::Orphaned => "orphaned",
        ActionState::Cancelled => "cancelled",
    }
}

fn parse_state(value: &str) -> Result<ActionState, CookerError> {
    match value {
        "planned" => Ok(ActionState::Planned),
        "simulated" => Ok(ActionState::Simulated),
        "submitted" => Ok(ActionState::Submitted),
        "confirmed" => Ok(ActionState::Confirmed),
        "rejected" => Ok(ActionState::Rejected),
        "failed" => Ok(ActionState::Failed),
        "expired" => Ok(ActionState::Expired),
        "unknown" => Ok(ActionState::Unknown),
        "orphaned" => Ok(ActionState::Orphaned),
        "cancelled" => Ok(ActionState::Cancelled),
        other => Err(CookerError::Store(format!(
            "unknown persisted action state {other}"
        ))),
    }
}

fn action_kind_name(kind: ActionKind) -> &'static str {
    match kind {
        ActionKind::NativeTransfer => "native_transfer",
        ActionKind::SplTransfer => "spl_transfer",
        ActionKind::JupiterSwap => "jupiter_swap",
        ActionKind::StakeLifecycle => "stake_lifecycle",
        ActionKind::Idle => "idle",
    }
}

fn session_state_name(state: SessionState) -> &'static str {
    match state {
        SessionState::Dormant => "dormant",
        SessionState::Active => "active",
        SessionState::Transacting => "transacting",
        SessionState::Holding => "holding",
        SessionState::CoolingDown => "cooling_down",
    }
}

fn confirmation_status_name(status: ConfirmationStatus) -> &'static str {
    match status {
        ConfirmationStatus::Missing => "missing",
        ConfirmationStatus::Processed => "processed",
        ConfirmationStatus::Confirmed => "confirmed",
        ConfirmationStatus::Finalized => "finalized",
        ConfirmationStatus::Failed => "failed",
    }
}

const fn audit_outcome_name(outcome: ConfirmationAuditOutcome) -> &'static str {
    match outcome {
        ConfirmationAuditOutcome::Verified => "verified",
        ConfirmationAuditOutcome::Orphaned => "orphaned",
    }
}

fn parse_audit_outcome(value: &str) -> Result<ConfirmationAuditOutcome, CookerError> {
    match value {
        "verified" => Ok(ConfirmationAuditOutcome::Verified),
        "orphaned" => Ok(ConfirmationAuditOutcome::Orphaned),
        other => Err(CookerError::Store(format!(
            "unknown persisted confirmation audit outcome {other}"
        ))),
    }
}

fn encode_json<T: Serialize + ?Sized>(value: &T) -> Result<String, CookerError> {
    serde_json::to_string(value)
        .map_err(|error| CookerError::Codec(format!("encode JSON: {error}")))
}

fn decode_json<T: DeserializeOwned>(value: &str) -> Result<T, CookerError> {
    serde_json::from_str(value).map_err(|error| CookerError::Codec(format!("decode JSON: {error}")))
}

fn decode_action_id(value: &str) -> Result<ActionId, CookerError> {
    decode_json(&encode_json(value)?)
}

fn parse_timestamp(value: &str) -> Result<DateTime<Utc>, CookerError> {
    DateTime::parse_from_rfc3339(value)
        .map(|parsed| parsed.with_timezone(&Utc))
        .map_err(|error| CookerError::Codec(format!("invalid persisted timestamp: {error}")))
}

fn parse_uuid(value: &str, field: &str) -> Result<Uuid, CookerError> {
    Uuid::parse_str(value)
        .map_err(|error| CookerError::Codec(format!("invalid persisted {field}: {error}")))
}

fn to_i64(value: u64, field: &str) -> Result<i64, CookerError> {
    i64::try_from(value).map_err(|error| {
        CookerError::Codec(format!("{field} does not fit SQLite integer: {error}"))
    })
}

fn from_i64(value: i64, field: &str) -> Result<u64, CookerError> {
    u64::try_from(value)
        .map_err(|error| CookerError::Codec(format!("invalid persisted {field}: {error}")))
}

#[cfg(test)]
pub(crate) fn execute_for_test(store: &Store, sql: &str) -> Result<usize, CookerError> {
    let connection = store.lock()?;
    connection
        .execute(sql, [])
        .map_err(|error| sqlite_error("execute test SQL", error))
}

#[cfg(test)]
pub(crate) fn pragma_for_test(
    store: &Store,
    name: &str,
) -> Result<rusqlite::types::Value, CookerError> {
    let connection = store.lock()?;
    connection
        .pragma_query_value(None, name, |row| row.get(0))
        .map_err(|error| sqlite_error("read test pragma", error))
}
