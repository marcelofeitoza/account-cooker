use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fs, io,
    path::PathBuf,
    sync::{Arc, Barrier},
    thread,
};

use chrono::{DateTime, TimeDelta, TimeZone, Utc};
use cooker_core::{
    ActionId, ActionKind, ActionLease, ActionPayload, ActionState, AgentId, AgentSnapshot,
    ChainReceipt, ConfirmationStatus, CookerError, LeaseId, PlannedAction, PreparedAction, RunId,
    SessionState, SimulationReceipt, StateStore, TraceEvent, TraceOutcome,
};
use proptest::{prelude::*, test_runner::TestCaseError};
use rusqlite::{Connection, params};
use tempfile::TempDir;
use uuid::Uuid;

use super::{ConfirmationAuditOutcome, RunRegistration, Store, StoreIdentity};
use crate::{migration, store};

type TestResult = Result<(), Box<dyn Error>>;

fn instant(day: u32, hour: u32, minute: u32) -> Result<DateTime<Utc>, io::Error> {
    Utc.with_ymd_and_hms(2026, 7, day, hour, minute, 0)
        .single()
        .ok_or_else(|| io::Error::other("invalid test timestamp"))
}

fn fixture() -> Result<(TempDir, PathBuf, Store), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("cooker.sqlite");
    let store = Store::open(&path, StoreIdentity::surfpool("test-surfnet")?)?;
    Ok((directory, path, store))
}

fn planned_action(
    run: RunId,
    agent: AgentId,
    sequence: u64,
    scheduled_at: DateTime<Utc>,
    payload: ActionPayload,
) -> PlannedAction {
    let model_version = "model-v1".to_owned();
    PlannedAction {
        id: ActionId::derive(run, agent, sequence, &model_version),
        run_id: run,
        agent_id: agent,
        sequence,
        model_version,
        scheduled_at,
        payload,
        max_fee_lamports: 5_000,
        max_account_creation_lamports: 0,
        created_at: scheduled_at - TimeDelta::minutes(1),
    }
}

fn native_action(
    run: RunId,
    agent: AgentId,
    sequence: u64,
    scheduled_at: DateTime<Utc>,
    lamports: u64,
) -> PlannedAction {
    planned_action(
        run,
        agent,
        sequence,
        scheduled_at,
        ActionPayload::NativeTransfer {
            destination: format!("destination-{sequence}"),
            lamports,
        },
    )
}

fn prepared(action: &PlannedAction, byte: u8) -> PreparedAction {
    PreparedAction {
        action_id: action.id.clone(),
        signature: format!("signature-{}-{byte}", action.id),
        transaction: vec![byte; 64],
        recent_blockhash: format!("blockhash-{byte}"),
        last_valid_block_height: 1234,
        expectations: Vec::new(),
    }
}

fn successful_simulation() -> SimulationReceipt {
    SimulationReceipt {
        succeeded: true,
        units_consumed: Some(12_345),
        logs: vec!["program success".to_owned()],
        error: None,
    }
}

fn receipt(
    action: &PlannedAction,
    prepared: &PreparedAction,
    observed_at: DateTime<Utc>,
) -> ChainReceipt {
    ChainReceipt {
        action_id: action.id.clone(),
        signature: prepared.signature.clone(),
        status: ConfirmationStatus::Confirmed,
        slot: Some(42),
        observed_at,
        postconditions_met: true,
        observations: Vec::new(),
        error: None,
    }
}

fn failed_receipt(
    action: &PlannedAction,
    prepared: &PreparedAction,
    observed_at: DateTime<Utc>,
) -> ChainReceipt {
    ChainReceipt {
        action_id: action.id.clone(),
        signature: prepared.signature.clone(),
        status: ConfirmationStatus::Failed,
        slot: Some(43),
        observed_at,
        postconditions_met: false,
        observations: Vec::new(),
        error: Some("fixture transaction failure".to_owned()),
    }
}

fn orphan_receipt(
    action: &PlannedAction,
    prepared: &PreparedAction,
    observed_at: DateTime<Utc>,
) -> ChainReceipt {
    ChainReceipt {
        action_id: action.id.clone(),
        signature: prepared.signature.clone(),
        status: ConfirmationStatus::Missing,
        slot: None,
        observed_at,
        postconditions_met: false,
        observations: Vec::new(),
        error: Some("fixture confirmation disappeared".to_owned()),
    }
}

fn terminal_trace(
    action: &PlannedAction,
    signature: Option<String>,
    observed_at: DateTime<Utc>,
    outcome: TraceOutcome,
) -> TraceEvent {
    let destination = match &action.payload {
        ActionPayload::NativeTransfer { destination, .. } => Some(destination.clone()),
        ActionPayload::SplTransfer {
            destination_owner, ..
        } => Some(destination_owner.clone()),
        ActionPayload::JupiterSwap { output_mint, .. } => Some(output_mint.clone()),
        ActionPayload::StakeLifecycle { stake_account, .. } => stake_account.clone(),
        ActionPayload::Idle => None,
    };
    TraceEvent {
        schema_version: TraceEvent::SCHEMA_VERSION,
        run_id: action.run_id,
        agent_id: action.agent_id,
        action_id: action.id.clone(),
        sequence: action.sequence,
        action_kind: action.payload.kind(),
        scheduled_at: action.scheduled_at,
        observed_at: Some(observed_at),
        amount: action.payload.input_amount(),
        destination,
        signature,
        outcome,
        attributes: BTreeMap::new(),
    }
}

fn first_lease(leases: &[ActionLease]) -> Result<&ActionLease, io::Error> {
    leases
        .first()
        .ok_or_else(|| io::Error::other("expected one lease"))
}

fn persist_submission(
    store: &Store,
    lease: &ActionLease,
    action: &PlannedAction,
    at: DateTime<Utc>,
    byte: u8,
) -> Result<PreparedAction, CookerError> {
    let signed = prepared(action, byte);
    store.record_prepared(lease, &signed, at)?;
    store.record_simulation(lease, &successful_simulation(), at)?;
    store.transition_action(
        lease,
        ActionState::Planned,
        ActionState::Simulated,
        at,
        None,
    )?;
    store.record_submission(lease, &signed.signature, at)?;
    store.transition_action(
        lease,
        ActionState::Simulated,
        ActionState::Submitted,
        at,
        None,
    )?;
    Ok(signed)
}

struct ObservabilityFixture {
    _directory: TempDir,
    store: Store,
    now: DateTime<Utc>,
    run_id: RunId,
    confirmed_id: ActionId,
}

#[allow(clippy::too_many_lines)]
fn observability_fixture() -> Result<ObservabilityFixture, Box<dyn Error>> {
    let (directory, _path, store) = fixture()?;
    let start = instant(17, 0, 0)?;
    let now = start + TimeDelta::hours(2);
    let run = RunId(Uuid::from_u128(900));
    let due_agent = AgentId(Uuid::from_u128(901));
    let future_agent = AgentId(Uuid::from_u128(902));
    let submitted = native_action(run, due_agent, 0, start, 10);
    let unknown = native_action(run, due_agent, 1, start, 11);
    let orphaned = native_action(run, due_agent, 2, start, 12);
    let confirmed = native_action(run, due_agent, 3, start, 13);
    for action in [&submitted, &unknown, &orphaned, &confirmed] {
        assert!(store.enqueue_action(action)?);
    }
    let claims = store.claim_due_actions("state-setup", start, now + TimeDelta::hours(1), 4)?;
    let by_action: BTreeMap<_, _> = claims
        .into_iter()
        .map(|lease| (lease.action_id.clone(), lease))
        .collect();
    for (index, action) in [&submitted, &unknown, &orphaned, &confirmed]
        .into_iter()
        .enumerate()
    {
        let lease = by_action
            .get(&action.id)
            .ok_or_else(|| io::Error::other("missing observability setup lease"))?;
        let signed = persist_submission(&store, lease, action, start, u8::try_from(index + 20)?)?;
        let mut released = false;
        match index {
            0 => {}
            1 => store.transition_action(
                lease,
                ActionState::Submitted,
                ActionState::Unknown,
                start,
                None,
            )?,
            2 | 3 => {
                let confirmed_receipt = receipt(action, &signed, start);
                store.record_receipt(lease, &confirmed_receipt)?;
                store.transition_action_with_trace(
                    lease,
                    ActionState::Submitted,
                    ActionState::Confirmed,
                    start,
                    None,
                    &terminal_trace(
                        action,
                        Some(signed.signature.clone()),
                        confirmed_receipt.observed_at,
                        TraceOutcome::Confirmed,
                    ),
                )?;
                if index == 2 {
                    let audit_receipt = orphan_receipt(action, &signed, start);
                    store.orphan_confirmation(
                        lease,
                        &audit_receipt,
                        start,
                        "fixture confirmation orphaned",
                        &terminal_trace(
                            action,
                            Some(signed.signature.clone()),
                            start,
                            TraceOutcome::Failed,
                        ),
                    )?;
                    released = true;
                }
            }
            _ => return Err(io::Error::other("unexpected setup index").into()),
        }
        if !released {
            store.release_lease(lease, start)?;
        }
    }

    let active = native_action(run, due_agent, 4, start + TimeDelta::minutes(1), 14);
    assert!(store.enqueue_action(&active)?);
    let active_claim = store.claim_due_actions(
        "active-worker",
        start + TimeDelta::minutes(5),
        now + TimeDelta::hours(1),
        1,
    )?;
    assert_eq!(active_claim.len(), 1);
    let expired = native_action(run, due_agent, 5, start + TimeDelta::minutes(2), 15);
    assert!(store.enqueue_action(&expired)?);
    let expired_claim = store.claim_due_actions(
        "expired-worker",
        start + TimeDelta::minutes(5),
        start + TimeDelta::minutes(10),
        1,
    )?;
    assert_eq!(expired_claim.len(), 1);
    let due = native_action(run, due_agent, 6, start + TimeDelta::minutes(3), 16);
    let future = native_action(run, future_agent, 0, now + TimeDelta::hours(1), 17);
    assert!(store.enqueue_action(&due)?);
    assert!(store.enqueue_action(&future)?);
    Ok(ObservabilityFixture {
        _directory: directory,
        store,
        now,
        run_id: run,
        confirmed_id: confirmed.id,
    })
}

#[test]
fn empty_database_migrates_and_pins_identity() -> TestResult {
    let (_directory, path, store) = fixture()?;
    assert_eq!(store.schema_version()?, 6);
    assert_eq!(store.journal_mode()?.to_ascii_lowercase(), "wal");
    assert_eq!(
        store::pragma_for_test(&store, "foreign_keys")?,
        rusqlite::types::Value::Integer(1)
    );
    assert_eq!(
        store::pragma_for_test(&store, "application_id")?,
        rusqlite::types::Value::Integer(migration::APPLICATION_ID)
    );
    store.verify_integrity()?;
    let database_id = store.database_id();
    drop(store);

    let reopened = Store::open(&path, StoreIdentity::surfpool("test-surfnet")?)?;
    assert_eq!(reopened.database_id(), database_id);
    assert!(matches!(
        Store::open(&path, StoreIdentity::surfpool("other-surfnet")?),
        Err(CookerError::InvalidConfig(_))
    ));
    assert!(matches!(
        Store::open(
            &path,
            StoreIdentity::new("different-network", "test-surfnet")?
        ),
        Err(CookerError::InvalidConfig(_))
    ));
    Ok(())
}

#[test]
fn read_only_open_never_creates_migrates_or_changes_store_files() -> TestResult {
    let (directory, path, store) = fixture()?;
    let database_id = store.database_id();
    drop(store);
    let before_database = fs::read(&path)?;
    let mut before_entries = fs::read_dir(directory.path())?
        .map(|entry| Ok(entry?.file_name()))
        .collect::<Result<Vec<_>, io::Error>>()?;
    before_entries.sort();

    for _ in 0..2 {
        let read_only = Store::open_read_only(&path, StoreIdentity::surfpool("test-surfnet")?)?;
        assert_eq!(read_only.database_id(), database_id);
        read_only.verify_integrity()?;
        let now = instant(17, 0, 0)?;
        assert_eq!(read_only.status_snapshot(now, None)?.total_actions, 0);
        assert_eq!(
            read_only
                .recovery_preview(now, None, 10)?
                .counts
                .expired_leases,
            0
        );
    }

    let mut after_entries = fs::read_dir(directory.path())?
        .map(|entry| Ok(entry?.file_name()))
        .collect::<Result<Vec<_>, io::Error>>()?;
    after_entries.sort();
    assert_eq!(fs::read(&path)?, before_database);
    assert_eq!(after_entries, before_entries);
    assert!(matches!(
        Store::open_read_only(&path, StoreIdentity::surfpool("wrong-surfnet")?),
        Err(CookerError::InvalidConfig(_))
    ));
    let missing = directory.path().join("missing.sqlite");
    assert!(matches!(
        Store::open_read_only(&missing, StoreIdentity::surfpool("test-surfnet")?),
        Err(CookerError::NotFound(_))
    ));
    assert!(!missing.exists());
    assert_eq!(fs::read(&path)?, before_database);
    Ok(())
}

#[test]
fn writable_reopen_of_current_store_is_byte_stable() -> TestResult {
    let (directory, path, store) = fixture()?;
    let database_id = store.database_id();
    drop(store);
    let before_database = fs::read(&path)?;
    let mut before_entries = fs::read_dir(directory.path())?
        .map(|entry| Ok(entry?.file_name()))
        .collect::<Result<Vec<_>, io::Error>>()?;
    before_entries.sort();

    for _ in 0..2 {
        let reopened = Store::open(&path, StoreIdentity::surfpool("test-surfnet")?)?;
        assert_eq!(reopened.database_id(), database_id);
        reopened.verify_integrity()?;
    }

    let mut after_entries = fs::read_dir(directory.path())?
        .map(|entry| Ok(entry?.file_name()))
        .collect::<Result<Vec<_>, io::Error>>()?;
    after_entries.sort();
    assert_eq!(fs::read(&path)?, before_database);
    assert_eq!(after_entries, before_entries);
    Ok(())
}

#[test]
fn policy_rewrite_is_atomic_auditable_and_forbidden_after_preparation() -> TestResult {
    let (_directory, _path, store) = fixture()?;
    let now = instant(17, 8, 0)?;
    let run = RunId(Uuid::from_u128(710));
    let agent = AgentId(Uuid::from_u128(711));
    let original = native_action(run, agent, 0, now, 10_000);
    assert!(store.enqueue_action(&original)?);
    let leases = store.claim_due_actions("rewrite-worker", now, now + TimeDelta::minutes(5), 1)?;
    let lease = first_lease(&leases)?;
    let replacement = ActionPayload::NativeTransfer {
        destination: "replacement-destination".to_owned(),
        lamports: 5_000,
    };

    let rewritten = store.rewrite_action(
        lease,
        &replacement,
        now + TimeDelta::seconds(1),
        "bounded_amount",
    )?;
    assert_eq!(rewritten.id, original.id);
    assert_eq!(rewritten.payload, replacement);
    assert_eq!(store.get_action(&original.id)?, rewritten);
    let events = store.action_events(&original.id)?;
    let rewrite = events
        .iter()
        .find(|event| event.kind == "policy_rewritten")
        .ok_or_else(|| io::Error::other("missing immutable rewrite event"))?;
    assert_eq!(rewrite.detail.as_deref(), Some("bounded_amount"));
    assert_eq!(rewrite.from_state, Some(ActionState::Planned));
    assert_eq!(rewrite.to_state, Some(ActionState::Planned));
    assert!(rewrite.payload.is_some());
    assert!(matches!(
        store.enqueue_action(&original),
        Err(CookerError::Store(_))
    ));

    store.record_prepared(
        lease,
        &prepared(&rewritten, 71),
        now + TimeDelta::seconds(2),
    )?;
    assert!(matches!(
        store.rewrite_action(
            lease,
            &ActionPayload::Idle,
            now + TimeDelta::seconds(3),
            "too_late",
        ),
        Err(CookerError::Store(_))
    ));
    Ok(())
}

#[test]
fn status_and_recovery_preview_are_complete_bounded_and_read_only() -> TestResult {
    let fixture = observability_fixture()?;
    let store = &fixture.store;
    let now = fixture.now;
    let early = now - TimeDelta::hours(1) - TimeDelta::minutes(54);
    let early_status = store.status_snapshot(early, None)?;
    assert_eq!(early_status.active_leases, 2);
    assert_eq!(early_status.expired_leases, 0);

    let status = store.status_snapshot(now, Some(now))?;
    assert_eq!(status.database_id, store.database_id());
    assert_eq!(status.identity, *store.identity());
    assert_eq!(status.observed_at, now);
    assert_eq!(status.total_agents, 2);
    assert_eq!(status.total_actions, 8);
    assert_eq!(status.actions_by_state.len(), 10);
    assert_eq!(status.actions_by_state[&ActionState::Planned], 4);
    assert_eq!(status.actions_by_state[&ActionState::Submitted], 1);
    assert_eq!(status.actions_by_state[&ActionState::Confirmed], 1);
    assert_eq!(status.actions_by_state[&ActionState::Unknown], 1);
    assert_eq!(status.actions_by_state[&ActionState::Orphaned], 1);
    for state in [
        ActionState::Simulated,
        ActionState::Rejected,
        ActionState::Failed,
        ActionState::Expired,
        ActionState::Cancelled,
    ] {
        assert_eq!(status.actions_by_state[&state], 0);
    }
    assert_eq!(status.active_leases, 1);
    assert_eq!(status.expired_leases, 1);
    assert_eq!(status.due_agents, 1);
    assert_eq!(status.due_actions, 2);
    assert_eq!(status.unresolved_actions, 3);
    assert_eq!(status.due_confirmation_audits, Some(1));
    assert_eq!(
        store.status_snapshot(now, None)?.due_confirmation_audits,
        None
    );
    let serialized = serde_json::to_value(&status)?;
    assert_eq!(serialized["actions_by_state"]["cancelled"], 0);
    assert_eq!(serialized["identity"]["network"], "surfpool");

    let preview = store.recovery_preview(now, Some(now), 2)?;
    assert_eq!(preview.counts.expired_leases, 1);
    assert_eq!(preview.counts.reconciliation_candidates, 3);
    assert_eq!(preview.counts.confirmation_audit_candidates, Some(1));
    assert_eq!(preview.expired_leases.len(), 1);
    assert_eq!(preview.reconciliation_candidates.len(), 2);
    assert_eq!(preview.confirmation_audit_candidates.len(), 1);
    assert_eq!(
        preview.confirmation_audit_candidates[0].action_id,
        fixture.confirmed_id
    );
    assert!(preview.confirmation_audit_candidates[0].signature.is_some());
    assert_eq!(preview, store.recovery_preview(now, Some(now), 2)?);
    let count_only = store.recovery_preview(now, Some(now), 0)?;
    assert_eq!(count_only.counts, preview.counts);
    assert!(count_only.expired_leases.is_empty());
    assert!(count_only.reconciliation_candidates.is_empty());
    assert!(count_only.confirmation_audit_candidates.is_empty());
    let no_audits = store.recovery_preview(now, None, 10)?;
    assert_eq!(no_audits.counts.confirmation_audit_candidates, None);
    assert!(no_audits.confirmation_audit_candidates.is_empty());
    assert!(serde_json::to_value(&preview)?.is_object());

    assert!(
        store
            .status_snapshot(now, Some(now + TimeDelta::seconds(1)))
            .is_err()
    );
    assert!(
        store
            .recovery_preview(now, Some(now + TimeDelta::seconds(1)), 10)
            .is_err()
    );
    assert_eq!(store.release_expired_leases(now)?, 1);
    let reconciliation = store.claim_reconciliation_candidates(
        "preview-proof-reconciler",
        now,
        now + TimeDelta::minutes(5),
        10,
    )?;
    assert_eq!(reconciliation.len(), 3);
    let audits = store.claim_confirmation_audit_candidates(
        "preview-proof-auditor",
        now,
        now + TimeDelta::minutes(5),
        now,
        10,
    )?;
    assert_eq!(audits.len(), 1);
    assert_eq!(audits[0].action_id, fixture.confirmed_id);
    assert_eq!(store.get_run(fixture.run_id)?.status, "active");
    Ok(())
}

#[test]
fn version_one_database_is_incrementally_migrated() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("v1.sqlite");
    let connection = Connection::open(&path)?;
    migration::configure(&connection)?;
    connection.pragma_update(None, "application_id", migration::APPLICATION_ID)?;
    connection.execute_batch(
        "CREATE TABLE schema_migrations (\
             version INTEGER PRIMARY KEY CHECK (version > 0),\
             name TEXT NOT NULL UNIQUE, checksum TEXT NOT NULL, applied_at TEXT NOT NULL\
         );",
    )?;
    let migration_sql = include_str!("../migrations/0001_initial.sql");
    connection.execute_batch(migration_sql)?;
    connection.execute(
        "INSERT INTO schema_migrations(version, name, checksum, applied_at)\
         VALUES (1, 'initial', ?1, ?2)",
        params![
            blake3::hash(migration_sql.as_bytes()).to_hex().to_string(),
            "2026-07-17T00:00:00.000000000Z"
        ],
    )?;
    connection.execute(
        "INSERT INTO store_identity(singleton, database_id, network, surfnet_id, created_at)\
         VALUES (1, ?1, 'surfpool', 'test-surfnet', ?2)",
        params![Uuid::new_v4().to_string(), "2026-07-17T00:00:00.000000000Z"],
    )?;
    connection.pragma_update(None, "user_version", 1_u32)?;
    drop(connection);

    let before_read_only = fs::read(&path)?;
    assert!(matches!(
        Store::open_read_only(&path, StoreIdentity::surfpool("test-surfnet")?),
        Err(CookerError::Store(_))
    ));
    assert_eq!(fs::read(&path)?, before_read_only);

    let store = Store::open(&path, StoreIdentity::surfpool("test-surfnet")?)?;
    assert_eq!(store.schema_version()?, 6);
    let run = RunId(Uuid::from_u128(1));
    let agent = AgentId(Uuid::from_u128(2));
    let action = native_action(run, agent, 0, instant(17, 1, 0)?, 1);
    assert!(store.enqueue_action(&action)?);
    assert!(
        store::execute_for_test(&store, "UPDATE action_events SET event_kind = 'tampered'")
            .is_err()
    );
    Ok(())
}

#[test]
fn deterministic_action_insertion_is_idempotent_and_collision_safe() -> TestResult {
    let (_directory, _path, store) = fixture()?;
    let action = native_action(
        RunId(Uuid::from_u128(10)),
        AgentId(Uuid::from_u128(11)),
        7,
        instant(17, 2, 0)?,
        50,
    );
    assert!(store.enqueue_action(&action)?);
    assert!(!store.enqueue_action(&action)?);
    assert_eq!(store.get_action(&action.id)?, action);

    let mut conflicting = action.clone();
    conflicting.payload = ActionPayload::NativeTransfer {
        destination: "different".to_owned(),
        lamports: 51,
    };
    assert!(matches!(
        store.enqueue_action(&conflicting),
        Err(CookerError::Store(_))
    ));
    assert_eq!(store.action_events(&action.id)?.len(), 1);
    Ok(())
}

#[test]
fn run_and_agent_metadata_survive_restart_and_reject_conflicts() -> TestResult {
    let (_directory, path, store) = fixture()?;
    let now = instant(17, 2, 30)?;
    let run = RunId(Uuid::from_u128(12));
    let agent = AgentId(Uuid::from_u128(13));
    let registration = RunRegistration {
        id: run,
        model_version: "model-v1".to_owned(),
        config: serde_json::json!({"agents": 1, "dry_run": true}),
        config_hash: "a1b2c3".to_owned(),
        seed_hash: "d4e5f6".to_owned(),
        created_at: now,
    };
    assert!(store.register_run(&registration)?);
    assert!(!store.register_run(&registration)?);
    let snapshot = AgentSnapshot {
        id: agent,
        run_id: run,
        next_sequence: 7,
        next_decision_at: now,
        budget_date: now.date_naive(),
        session_state: SessionState::Active,
        last_action_at: Some(now - TimeDelta::minutes(1)),
        remaining_daily_budget: 1000,
        model_version: "model-v1".to_owned(),
    };
    store.upsert_agent(&snapshot, now)?;
    store.transition_run(run, "active", "completed", now + TimeDelta::minutes(1))?;
    drop(store);

    let reopened = Store::open(&path, StoreIdentity::surfpool("test-surfnet")?)?;
    assert_eq!(reopened.get_agent(agent)?, snapshot);
    let persisted_run = reopened.get_run(run)?;
    assert_eq!(persisted_run.status, "completed");
    assert_eq!(persisted_run.config, Some(registration.config.clone()));
    let mut conflicting = registration;
    conflicting.config_hash = "ffffff".to_owned();
    assert!(matches!(
        reopened.register_run(&conflicting),
        Err(CookerError::Store(_))
    ));
    let conflicting_agent = AgentSnapshot {
        run_id: RunId(Uuid::from_u128(14)),
        ..snapshot
    };
    assert!(matches!(
        reopened.upsert_agent(&conflicting_agent, now),
        Err(CookerError::Store(_))
    ));
    Ok(())
}

#[test]
fn due_agent_cursor_uses_sequence_compare_and_swap() -> TestResult {
    let (_directory, _path, store) = fixture()?;
    let now = instant(17, 2, 45)?;
    let run = RunId(Uuid::from_u128(16));
    let agent = AgentId(Uuid::from_u128(17));
    let snapshot = AgentSnapshot {
        id: agent,
        run_id: run,
        next_sequence: 0,
        next_decision_at: now,
        budget_date: now.date_naive(),
        session_state: SessionState::Active,
        last_action_at: None,
        remaining_daily_budget: 1000,
        model_version: "model-v1".to_owned(),
    };
    store.upsert_agent(&snapshot, now)?;
    assert_eq!(store.due_agents(run, now, 10)?, vec![snapshot.clone()]);

    let mut winner = snapshot.clone();
    winner.next_sequence = 1;
    winner.next_decision_at = now + TimeDelta::minutes(10);
    winner.last_action_at = Some(winner.next_decision_at);
    assert!(store.advance_agent(0, &winner, now + TimeDelta::seconds(1))?);

    let mut stale = snapshot;
    stale.next_sequence = 1;
    stale.next_decision_at = now + TimeDelta::minutes(20);
    assert!(!store.advance_agent(0, &stale, now + TimeDelta::seconds(2))?);
    assert_eq!(store.get_agent(agent)?, winner);
    assert!(
        store
            .due_agents(run, now + TimeDelta::minutes(9), 10)?
            .is_empty()
    );
    assert_eq!(
        store
            .due_agents(run, now + TimeDelta::minutes(10), 10)?
            .len(),
        1
    );
    Ok(())
}

#[test]
fn claims_are_atomic_exclusive_and_expire() -> TestResult {
    let (_directory, path, store) = fixture()?;
    let now = instant(17, 3, 0)?;
    let run = RunId(Uuid::from_u128(20));
    let agent = AgentId(Uuid::from_u128(21));
    let first = native_action(run, agent, 0, now, 10);
    let second = native_action(run, agent, 1, now, 20);
    let future = native_action(run, agent, 2, now + TimeDelta::hours(1), 30);
    for action in [&first, &second, &future] {
        assert!(store.enqueue_action(action)?);
    }
    let first_claim = store.claim_due_actions("worker-a", now, now + TimeDelta::minutes(5), 1)?;
    assert_eq!(first_claim.len(), 1);

    let peer = Store::open(&path, StoreIdentity::surfpool("test-surfnet")?)?;
    let second_claim = peer.claim_due_actions("worker-b", now, now + TimeDelta::minutes(5), 10)?;
    assert_eq!(second_claim.len(), 1);
    assert_ne!(
        first_lease(&first_claim)?.action_id,
        first_lease(&second_claim)?.action_id
    );

    let after_expiry = now + TimeDelta::minutes(6);
    let reclaimed = peer.claim_due_actions(
        "worker-c",
        after_expiry,
        after_expiry + TimeDelta::minutes(5),
        10,
    )?;
    assert_eq!(reclaimed.len(), 2);
    let reclaimed_ids: BTreeSet<_> = reclaimed
        .iter()
        .map(|lease| lease.action_id.clone())
        .collect();
    assert!(reclaimed_ids.contains(&first.id));
    assert!(reclaimed_ids.contains(&second.id));
    assert!(!reclaimed_ids.contains(&future.id));
    Ok(())
}

#[test]
fn policy_deferral_survives_restart_and_prevents_early_reclaim() -> TestResult {
    let (_directory, path, store) = fixture()?;
    let now = instant(17, 3, 0)?;
    let action = native_action(
        RunId(Uuid::from_u128(24)),
        AgentId(Uuid::from_u128(25)),
        0,
        now,
        10,
    );
    assert!(store.enqueue_action(&action)?);
    let leases = store.claim_due_actions("worker-a", now, now + TimeDelta::hours(2), 1)?;
    let lease = first_lease(&leases)?;
    let recorded_at = now + TimeDelta::minutes(1);
    let eligible_at = now + TimeDelta::hours(1);
    store.defer_action(lease, eligible_at, recorded_at, "daily_budget_exceeded")?;
    assert!(
        store
            .claim_due_actions(
                "worker-b",
                now + TimeDelta::minutes(30),
                now + TimeDelta::minutes(35),
                1,
            )?
            .is_empty()
    );
    drop(store);

    let reopened = Store::open(&path, StoreIdentity::surfpool("test-surfnet")?)?;
    assert!(
        reopened
            .claim_due_actions(
                "worker-c",
                eligible_at - TimeDelta::seconds(1),
                eligible_at + TimeDelta::minutes(4),
                1,
            )?
            .is_empty()
    );
    let reclaimed = reopened.claim_due_actions(
        "worker-c",
        eligible_at,
        eligible_at + TimeDelta::minutes(5),
        1,
    )?;
    assert_eq!(first_lease(&reclaimed)?.action_id, action.id);
    assert!(
        reopened
            .action_events(&action.id)?
            .iter()
            .any(|event| event.kind == "policy_deferred")
    );
    Ok(())
}

#[test]
fn concurrent_store_instances_never_double_claim() -> TestResult {
    let (_directory, path, store) = fixture()?;
    let now = instant(17, 4, 0)?;
    let run = RunId(Uuid::from_u128(30));
    let agent = AgentId(Uuid::from_u128(31));
    for sequence in 0..20 {
        let action = native_action(run, agent, sequence, now, sequence + 1);
        assert!(store.enqueue_action(&action)?);
    }
    drop(store);

    let barrier = Arc::new(Barrier::new(3));
    let mut handles = Vec::new();
    for worker in ["worker-a", "worker-b"] {
        let path = path.clone();
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(
            move || -> Result<Vec<ActionId>, CookerError> {
                let store = Store::open(&path, StoreIdentity::surfpool("test-surfnet")?)?;
                barrier.wait();
                store
                    .claim_due_actions(worker, now, now + TimeDelta::minutes(10), 20)
                    .map(|leases| leases.into_iter().map(|lease| lease.action_id).collect())
            },
        ));
    }
    barrier.wait();
    let mut all = Vec::new();
    for handle in handles {
        let claimed = handle
            .join()
            .map_err(|_| io::Error::other("claim thread panicked"))??;
        all.extend(claimed);
    }
    let unique: BTreeSet<_> = all.iter().cloned().collect();
    assert_eq!(all.len(), 20);
    assert_eq!(unique.len(), 20);
    Ok(())
}

#[test]
fn every_mutation_rejects_forged_released_and_expired_leases() -> TestResult {
    let (_directory, _path, store) = fixture()?;
    let now = instant(17, 5, 0)?;
    let action = native_action(
        RunId(Uuid::from_u128(40)),
        AgentId(Uuid::from_u128(41)),
        0,
        now,
        10,
    );
    assert!(store.enqueue_action(&action)?);
    let leases = store.claim_due_actions("worker", now, now + TimeDelta::minutes(5), 1)?;
    let lease = first_lease(&leases)?.clone();

    let mut forged_worker = lease.clone();
    forged_worker.worker_id = "attacker".to_owned();
    assert!(matches!(
        store.transition_action(
            &forged_worker,
            ActionState::Planned,
            ActionState::Simulated,
            now + TimeDelta::minutes(1),
            None
        ),
        Err(CookerError::LeaseConflict(_))
    ));

    let mut forged_id = lease.clone();
    forged_id.id = LeaseId::new();
    assert!(matches!(
        store.record_prepared(&forged_id, &prepared(&action, 1), now),
        Err(CookerError::LeaseConflict(_))
    ));

    let mut forged_expiry = lease.clone();
    forged_expiry.expires_at += TimeDelta::seconds(1);
    assert!(matches!(
        store.record_simulation(&forged_expiry, &successful_simulation(), now),
        Err(CookerError::LeaseConflict(_))
    ));

    let other_action = native_action(action.run_id, action.agent_id, 1, now, 1);
    let mut forged_action = lease.clone();
    forged_action.action_id = other_action.id;
    assert!(matches!(
        store.transition_action(
            &forged_action,
            ActionState::Planned,
            ActionState::Simulated,
            now,
            None
        ),
        Err(CookerError::LeaseConflict(_))
    ));

    assert!(matches!(
        store.record_prepared(&lease, &prepared(&action, 1), now + TimeDelta::minutes(5)),
        Err(CookerError::LeaseConflict(_))
    ));

    store.release_lease_at(&lease, now + TimeDelta::minutes(1))?;
    assert!(matches!(
        store.record_prepared(&lease, &prepared(&action, 1), now + TimeDelta::minutes(2)),
        Err(CookerError::LeaseConflict(_))
    ));
    Ok(())
}

#[test]
fn terminal_transition_and_trace_commit_or_roll_back_together() -> TestResult {
    let (_directory, _path, store) = fixture()?;
    let now = instant(17, 5, 0)?;
    let action = native_action(
        RunId(Uuid::from_u128(45)),
        AgentId(Uuid::from_u128(46)),
        0,
        now,
        500,
    );
    assert!(store.enqueue_action(&action)?);
    let leases = store.claim_due_actions("worker", now, now + TimeDelta::hours(1), 1)?;
    let lease = first_lease(&leases)?;
    let mut trace = TraceEvent {
        schema_version: TraceEvent::SCHEMA_VERSION,
        run_id: action.run_id,
        agent_id: action.agent_id,
        action_id: action.id.clone(),
        sequence: action.sequence + 1,
        action_kind: action.payload.kind(),
        scheduled_at: action.scheduled_at,
        observed_at: Some(now + TimeDelta::minutes(1)),
        amount: action.payload.input_amount(),
        destination: Some("destination-0".to_owned()),
        signature: None,
        outcome: TraceOutcome::Rejected,
        attributes: BTreeMap::new(),
    };

    assert!(
        store
            .transition_action_with_trace(
                lease,
                ActionState::Planned,
                ActionState::Rejected,
                now + TimeDelta::minutes(1),
                Some("test rejection"),
                &trace,
            )
            .is_err()
    );
    assert_eq!(store.get_action_state(&action.id)?, ActionState::Planned);
    assert!(store.traces_for_run(action.run_id)?.is_empty());
    assert!(
        store
            .action_events(&action.id)?
            .iter()
            .all(|event| event.to_state != Some(ActionState::Rejected))
    );

    trace.sequence = action.sequence;
    store.transition_action_with_trace(
        lease,
        ActionState::Planned,
        ActionState::Rejected,
        now + TimeDelta::minutes(1),
        Some("test rejection"),
        &trace,
    )?;
    assert_eq!(store.get_action_state(&action.id)?, ActionState::Rejected);
    assert_eq!(store.traces_for_run(action.run_id)?, vec![trace]);
    Ok(())
}

#[test]
#[allow(clippy::too_many_lines)]
fn signed_bytes_receipts_events_and_traces_are_restart_safe_and_idempotent() -> TestResult {
    let (_directory, path, store) = fixture()?;
    let now = instant(17, 6, 0)?;
    let action = native_action(
        RunId(Uuid::from_u128(50)),
        AgentId(Uuid::from_u128(51)),
        0,
        now,
        500,
    );
    assert!(store.enqueue_action(&action)?);
    let leases = store.claim_due_actions("worker", now, now + TimeDelta::hours(1), 1)?;
    let lease = first_lease(&leases)?;
    let prepared_action = prepared(&action, 7);
    let simulation = successful_simulation();

    assert!(
        store
            .record_submission(lease, &prepared_action.signature, now)
            .is_err()
    );
    store.record_prepared(lease, &prepared_action, now + TimeDelta::minutes(1))?;
    store.record_prepared(lease, &prepared_action, now + TimeDelta::minutes(1))?;
    assert!(
        store
            .record_prepared(lease, &prepared(&action, 8), now + TimeDelta::minutes(1))
            .is_err()
    );
    store.record_simulation(lease, &simulation, now + TimeDelta::minutes(2))?;
    store.record_simulation(lease, &simulation, now + TimeDelta::minutes(2))?;
    store.transition_action(
        lease,
        ActionState::Planned,
        ActionState::Simulated,
        now + TimeDelta::minutes(3),
        Some("simulation passed"),
    )?;
    store.record_submission(
        lease,
        &prepared_action.signature,
        now + TimeDelta::minutes(4),
    )?;
    store.record_submission(
        lease,
        &prepared_action.signature,
        now + TimeDelta::minutes(4),
    )?;
    store.transition_action(
        lease,
        ActionState::Simulated,
        ActionState::Submitted,
        now + TimeDelta::minutes(5),
        None,
    )?;
    let receipt = receipt(&action, &prepared_action, now + TimeDelta::minutes(6));
    store.record_receipt(lease, &receipt)?;
    store.record_receipt(lease, &receipt)?;
    let trace = TraceEvent {
        schema_version: TraceEvent::SCHEMA_VERSION,
        run_id: action.run_id,
        agent_id: action.agent_id,
        action_id: action.id.clone(),
        sequence: action.sequence,
        action_kind: ActionKind::NativeTransfer,
        scheduled_at: action.scheduled_at,
        observed_at: Some(receipt.observed_at),
        amount: 500,
        destination: Some("destination-0".to_owned()),
        signature: Some(prepared_action.signature.clone()),
        outcome: TraceOutcome::Confirmed,
        attributes: BTreeMap::new(),
    };
    store.transition_action_with_trace(
        lease,
        ActionState::Submitted,
        ActionState::Confirmed,
        now + TimeDelta::minutes(7),
        None,
        &trace,
    )?;
    store.append_trace(&trace)?;
    store.append_trace(&trace)?;
    drop(store);

    let reopened = Store::open(&path, StoreIdentity::surfpool("test-surfnet")?)?;
    assert_eq!(
        reopened.get_action_state(&action.id)?,
        ActionState::Confirmed
    );
    assert_eq!(
        reopened.get_prepared(&action.id)?,
        Some(prepared_action.clone())
    );
    assert_eq!(
        reopened.get_submission_signature(&action.id)?,
        Some(prepared_action.signature.clone())
    );
    let recovery = reopened.recovery_record(&action.id, now + TimeDelta::minutes(8))?;
    assert_eq!(recovery.prepared, Some(prepared_action));
    assert_eq!(recovery.simulations.len(), 1);
    assert_eq!(recovery.receipts, vec![receipt]);
    assert_eq!(reopened.traces_for_run(action.run_id)?, vec![trace]);
    assert!(recovery.events.len() >= 9);
    assert!(store::execute_for_test(&reopened, "DELETE FROM traces WHERE trace_id = 1").is_err());
    assert!(
        store::execute_for_test(
            &reopened,
            "UPDATE prepared_transactions SET signature = 'tampered'"
        )
        .is_err()
    );
    Ok(())
}

#[test]
#[allow(clippy::too_many_lines)]
fn confirmation_audit_claims_enforce_lease_time_and_append_immutable_history() -> TestResult {
    let (_directory, _path, store) = fixture()?;
    let now = instant(17, 7, 0)?;
    let action = native_action(
        RunId(Uuid::from_u128(55)),
        AgentId(Uuid::from_u128(56)),
        0,
        now,
        500,
    );
    assert!(store.enqueue_action(&action)?);
    let initial = store.claim_due_actions("worker", now, now + TimeDelta::hours(1), 1)?;
    let initial_lease = first_lease(&initial)?;
    let signed = prepared(&action, 11);
    store.record_prepared(initial_lease, &signed, now)?;
    store.record_simulation(initial_lease, &successful_simulation(), now)?;
    store.transition_action(
        initial_lease,
        ActionState::Planned,
        ActionState::Simulated,
        now,
        None,
    )?;
    store.record_submission(initial_lease, &signed.signature, now)?;
    store.transition_action(
        initial_lease,
        ActionState::Simulated,
        ActionState::Submitted,
        now,
        None,
    )?;
    let initial_receipt = receipt(&action, &signed, now);
    store.record_receipt(initial_lease, &initial_receipt)?;
    store.transition_action_with_trace(
        initial_lease,
        ActionState::Submitted,
        ActionState::Confirmed,
        now,
        None,
        &terminal_trace(
            &action,
            Some(signed.signature.clone()),
            now,
            TraceOutcome::Confirmed,
        ),
    )?;
    store.release_lease(initial_lease, now)?;

    let audit_time = now + TimeDelta::hours(2);
    let claimed = store.claim_confirmation_audit_candidates(
        "auditor",
        audit_time,
        audit_time + TimeDelta::minutes(5),
        audit_time,
        1,
    )?;
    let audit_lease = first_lease(&claimed)?;
    let audit_receipt = receipt(&action, &signed, audit_time);
    let mut forged = audit_lease.clone();
    forged.worker_id = "other-auditor".to_owned();
    assert!(matches!(
        store.complete_confirmation_audit(&forged, &audit_receipt, audit_time),
        Err(CookerError::LeaseConflict(_))
    ));
    assert!(store.confirmation_audits(&action.id)?.is_empty());

    store.complete_confirmation_audit(audit_lease, &audit_receipt, audit_time)?;
    let history = store.confirmation_audits(&action.id)?;
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].outcome, ConfirmationAuditOutcome::Verified);
    assert_eq!(history[0].audited_at, audit_time);
    assert!(matches!(
        store.complete_confirmation_audit(audit_lease, &audit_receipt, audit_time),
        Err(CookerError::LeaseConflict(_))
    ));
    assert!(
        store
            .claim_confirmation_audit_candidates(
                "immediate-auditor",
                audit_time,
                audit_time + TimeDelta::minutes(5),
                audit_time,
                1,
            )?
            .is_empty()
    );
    assert!(
        store::execute_for_test(
            &store,
            "UPDATE confirmation_audits SET outcome = 'orphaned'"
        )
        .is_err()
    );

    let later = audit_time + TimeDelta::hours(1);
    let claimed = store.claim_confirmation_audit_candidates(
        "expired-auditor",
        later,
        later + TimeDelta::minutes(1),
        later,
        1,
    )?;
    let expired = first_lease(&claimed)?;
    let later_receipt = receipt(&action, &signed, later);
    assert!(matches!(
        store.complete_confirmation_audit(expired, &later_receipt, expired.expires_at),
        Err(CookerError::LeaseConflict(_))
    ));
    assert_eq!(store.confirmation_audits(&action.id)?.len(), 1);
    Ok(())
}

#[test]
fn crash_window_submission_is_reconciled_and_never_reclaimed_as_due() -> TestResult {
    let (_directory, path, store) = fixture()?;
    let now = instant(17, 8, 0)?;
    let action = native_action(
        RunId(Uuid::from_u128(60)),
        AgentId(Uuid::from_u128(61)),
        0,
        now,
        100,
    );
    assert!(store.enqueue_action(&action)?);
    let leases = store.claim_due_actions("runtime", now, now + TimeDelta::minutes(2), 1)?;
    let lease = first_lease(&leases)?;
    let prepared = prepared(&action, 9);
    store.record_prepared(lease, &prepared, now)?;
    store.record_simulation(lease, &successful_simulation(), now)?;
    store.transition_action(
        lease,
        ActionState::Planned,
        ActionState::Simulated,
        now,
        None,
    )?;
    store.record_submission(lease, &prepared.signature, now + TimeDelta::minutes(1))?;
    drop(store);

    let restarted = Store::open(&path, StoreIdentity::surfpool("test-surfnet")?)?;
    let after_expiry = now + TimeDelta::minutes(3);
    assert!(
        restarted
            .claim_due_actions(
                "normal-worker",
                after_expiry,
                after_expiry + TimeDelta::minutes(5),
                10
            )?
            .is_empty()
    );
    let reconciliation = restarted.claim_reconciliation_candidates(
        "reconciler",
        after_expiry,
        after_expiry + TimeDelta::minutes(5),
        10,
    )?;
    assert_eq!(reconciliation.len(), 1);
    assert_eq!(first_lease(&reconciliation)?.action_id, action.id);
    assert!(
        restarted
            .claim_reconciliation_candidates(
                "other-reconciler",
                after_expiry,
                after_expiry + TimeDelta::minutes(5),
                10
            )?
            .is_empty()
    );
    Ok(())
}

#[test]
#[allow(clippy::too_many_lines)]
fn restart_restores_every_lifecycle_state_and_rollback_edges() -> TestResult {
    let (_directory, path, store) = fixture()?;
    let now = instant(17, 9, 0)?;
    let run = RunId(Uuid::from_u128(70));
    let agent = AgentId(Uuid::from_u128(71));
    let actions: Vec<_> = (0..10)
        .map(|sequence| native_action(run, agent, sequence, now, sequence + 1))
        .collect();
    for action in &actions {
        assert!(store.enqueue_action(action)?);
    }
    let leases = store.claim_due_actions("worker", now, now + TimeDelta::days(1), 10)?;
    let by_action: BTreeMap<_, _> = leases
        .into_iter()
        .map(|lease| (lease.action_id.clone(), lease))
        .collect();
    let expected_states = [
        ActionState::Planned,
        ActionState::Simulated,
        ActionState::Rejected,
        ActionState::Expired,
        ActionState::Cancelled,
        ActionState::Submitted,
        ActionState::Confirmed,
        ActionState::Failed,
        ActionState::Unknown,
        ActionState::Orphaned,
    ];
    for (index, action) in actions.iter().enumerate() {
        let lease = by_action
            .get(&action.id)
            .ok_or_else(|| io::Error::other("missing lifecycle lease"))?;
        let at = now + TimeDelta::seconds(i64::try_from(index + 1)?);
        match index {
            0 => {}
            1 => {
                let signed = prepared(action, 21);
                store.record_prepared(lease, &signed, at)?;
                store.record_simulation(lease, &successful_simulation(), at)?;
                store.transition_action(
                    lease,
                    ActionState::Planned,
                    ActionState::Simulated,
                    at,
                    None,
                )?;
            }
            2..=4 => {
                let next = expected_states[index];
                store.transition_action_with_trace(
                    lease,
                    ActionState::Planned,
                    next,
                    at,
                    None,
                    &terminal_trace(action, None, at, TraceOutcome::Rejected),
                )?;
            }
            5 => {
                persist_submission(&store, lease, action, at, 25)?;
            }
            6 => {
                let signed = persist_submission(&store, lease, action, at, 26)?;
                let observed = receipt(action, &signed, at);
                store.record_receipt(lease, &observed)?;
                store.transition_action_with_trace(
                    lease,
                    ActionState::Submitted,
                    ActionState::Confirmed,
                    at,
                    None,
                    &terminal_trace(
                        action,
                        Some(signed.signature.clone()),
                        at,
                        TraceOutcome::Confirmed,
                    ),
                )?;
            }
            7 => {
                let signed = persist_submission(&store, lease, action, at, 27)?;
                let observed = failed_receipt(action, &signed, at);
                store.record_receipt(lease, &observed)?;
                store.transition_action_with_trace(
                    lease,
                    ActionState::Submitted,
                    ActionState::Failed,
                    at,
                    Some("fixture transaction failure"),
                    &terminal_trace(
                        action,
                        Some(signed.signature.clone()),
                        at,
                        TraceOutcome::Failed,
                    ),
                )?;
            }
            8 => {
                persist_submission(&store, lease, action, at, 28)?;
                store.transition_action(
                    lease,
                    ActionState::Submitted,
                    ActionState::Unknown,
                    at,
                    None,
                )?;
            }
            9 => {
                let signed = persist_submission(&store, lease, action, at, 29)?;
                let observed = receipt(action, &signed, at);
                store.record_receipt(lease, &observed)?;
                store.transition_action_with_trace(
                    lease,
                    ActionState::Submitted,
                    ActionState::Confirmed,
                    at,
                    None,
                    &terminal_trace(
                        action,
                        Some(signed.signature.clone()),
                        at,
                        TraceOutcome::Confirmed,
                    ),
                )?;
                store.orphan_confirmation(
                    lease,
                    &orphan_receipt(action, &signed, at),
                    at,
                    "fixture confirmation orphaned",
                    &terminal_trace(action, Some(signed.signature), at, TraceOutcome::Failed),
                )?;
            }
            _ => return Err(io::Error::other("unexpected lifecycle fixture index").into()),
        }
    }
    drop(store);

    let reopened = Store::open(&path, StoreIdentity::surfpool("test-surfnet")?)?;
    for (action, expected) in actions.iter().zip(expected_states) {
        assert_eq!(reopened.get_action_state(&action.id)?, expected);
        assert_eq!(reopened.recovery_record(&action.id, now)?.state, expected);
    }
    let orphaned = actions
        .last()
        .ok_or_else(|| io::Error::other("missing orphaned action"))?;
    let recovery_time = now + TimeDelta::minutes(1);
    let reconciliation = reopened.claim_reconciliation_candidates(
        "rollback-reconciler",
        recovery_time,
        recovery_time + TimeDelta::minutes(5),
        10,
    )?;
    let orphaned_lease = reconciliation
        .iter()
        .find(|lease| lease.action_id == orphaned.id)
        .ok_or_else(|| io::Error::other("missing orphaned reconciliation lease"))?;
    let signed = reopened
        .get_prepared(&orphaned.id)?
        .ok_or_else(|| io::Error::other("missing orphaned prepared transaction"))?;
    let recovered_receipt = receipt(orphaned, &signed, recovery_time);
    reopened.record_receipt(orphaned_lease, &recovered_receipt)?;
    reopened.transition_action_with_trace(
        orphaned_lease,
        ActionState::Orphaned,
        ActionState::Confirmed,
        recovery_time,
        Some("rollback recovered"),
        &terminal_trace(
            orphaned,
            Some(signed.signature),
            recovery_time,
            TraceOutcome::Confirmed,
        ),
    )?;
    assert_eq!(
        reopened.get_action_state(&orphaned.id)?,
        ActionState::Confirmed
    );
    Ok(())
}

#[test]
#[allow(clippy::too_many_lines)]
fn lifecycle_transitions_require_durable_artifacts_and_atomic_terminal_traces() -> TestResult {
    let (_directory, _path, store) = fixture()?;
    let now = instant(17, 9, 30)?;
    let action = native_action(
        RunId(Uuid::from_u128(75)),
        AgentId(Uuid::from_u128(76)),
        0,
        now,
        500,
    );
    assert!(store.enqueue_action(&action)?);
    let claims = store.claim_due_actions("worker", now, now + TimeDelta::hours(1), 1)?;
    let lease = first_lease(&claims)?;

    assert!(
        store
            .transition_action(
                lease,
                ActionState::Planned,
                ActionState::Simulated,
                now,
                None,
            )
            .is_err()
    );
    let signed = prepared(&action, 30);
    store.record_prepared(lease, &signed, now)?;
    store.record_simulation(lease, &successful_simulation(), now)?;
    store.transition_action(
        lease,
        ActionState::Planned,
        ActionState::Simulated,
        now,
        None,
    )?;
    assert!(
        store
            .transition_action(
                lease,
                ActionState::Simulated,
                ActionState::Submitted,
                now,
                None,
            )
            .is_err()
    );
    store.record_submission(lease, &signed.signature, now)?;
    store.transition_action(
        lease,
        ActionState::Simulated,
        ActionState::Submitted,
        now,
        None,
    )?;
    assert!(
        store
            .transition_action(
                lease,
                ActionState::Submitted,
                ActionState::Confirmed,
                now,
                None,
            )
            .is_err()
    );
    assert!(
        store
            .transition_action_with_trace(
                lease,
                ActionState::Submitted,
                ActionState::Confirmed,
                now,
                None,
                &terminal_trace(
                    &action,
                    Some(signed.signature.clone()),
                    now,
                    TraceOutcome::Confirmed,
                ),
            )
            .is_err()
    );
    let observed = receipt(&action, &signed, now);
    store.record_receipt(lease, &observed)?;
    assert!(
        store
            .transition_action(
                lease,
                ActionState::Submitted,
                ActionState::Confirmed,
                now,
                None,
            )
            .is_err()
    );
    store.transition_action_with_trace(
        lease,
        ActionState::Submitted,
        ActionState::Confirmed,
        now,
        None,
        &terminal_trace(
            &action,
            Some(signed.signature),
            now,
            TraceOutcome::Confirmed,
        ),
    )?;
    Ok(())
}

#[test]
#[allow(clippy::too_many_lines)]
fn budget_usage_counts_confirmed_and_conservatively_pending_native_units() -> TestResult {
    let (_directory, _path, store) = fixture()?;
    let yesterday = instant(16, 10, 0)?;
    let today = instant(17, 10, 0)?;
    let run = RunId(Uuid::from_u128(80));
    let agent = AgentId(Uuid::from_u128(81));
    let confirmed_yesterday = native_action(run, agent, 0, yesterday, 100);
    let pending_today = native_action(run, agent, 1, today, 200);
    let unknown_today = planned_action(
        run,
        agent,
        2,
        today,
        ActionPayload::StakeLifecycle {
            operation: cooker_core::StakeOperation::Enter,
            lamports: 300,
            stake_account: None,
        },
    );
    let rejected_today = native_action(run, agent, 3, today, 400);
    let spl_today = planned_action(
        run,
        agent,
        4,
        today,
        ActionPayload::SplTransfer {
            mint: "mint".to_owned(),
            destination_owner: "owner".to_owned(),
            amount: 999,
        },
    );
    let actions = [
        &confirmed_yesterday,
        &pending_today,
        &unknown_today,
        &rejected_today,
        &spl_today,
    ];
    for action in actions {
        assert!(store.enqueue_action(action)?);
    }
    let prior_claim = store.claim_due_actions(
        "prior-day-worker",
        yesterday,
        yesterday + TimeDelta::hours(1),
        1,
    )?;
    let confirmed_lease = first_lease(&prior_claim)?;
    assert_eq!(confirmed_lease.action_id, confirmed_yesterday.id);
    let confirmed_signed =
        persist_submission(&store, confirmed_lease, &confirmed_yesterday, yesterday, 31)?;
    let confirmed_receipt = receipt(&confirmed_yesterday, &confirmed_signed, yesterday);
    store.record_receipt(confirmed_lease, &confirmed_receipt)?;
    store.transition_action_with_trace(
        confirmed_lease,
        ActionState::Submitted,
        ActionState::Confirmed,
        yesterday,
        None,
        &terminal_trace(
            &confirmed_yesterday,
            Some(confirmed_signed.signature),
            yesterday,
            TraceOutcome::Confirmed,
        ),
    )?;
    store.release_lease(confirmed_lease, yesterday)?;

    let claims = store.claim_due_actions("worker", today, today + TimeDelta::hours(1), 10)?;
    let leases: BTreeMap<_, _> = claims
        .into_iter()
        .map(|lease| (lease.action_id.clone(), lease))
        .collect();
    let lease_for = |action: &PlannedAction| -> Result<&ActionLease, Box<dyn Error>> {
        let lease = leases
            .get(&action.id)
            .ok_or_else(|| io::Error::other("missing budget action lease"))?;
        Ok(lease)
    };
    persist_submission(
        &store,
        lease_for(&pending_today)?,
        &pending_today,
        today,
        32,
    )?;
    let unknown_lease = lease_for(&unknown_today)?;
    persist_submission(&store, unknown_lease, &unknown_today, today, 33)?;
    store.transition_action(
        unknown_lease,
        ActionState::Submitted,
        ActionState::Unknown,
        today,
        None,
    )?;
    store.transition_action_with_trace(
        lease_for(&rejected_today)?,
        ActionState::Planned,
        ActionState::Rejected,
        today,
        None,
        &terminal_trace(&rejected_today, None, today, TraceOutcome::Rejected),
    )?;
    persist_submission(&store, lease_for(&spl_today)?, &spl_today, today, 34)?;

    let usage = store.budget_usage(agent, today)?;
    assert_eq!(usage.spent_today_lamports, 15_500);
    assert_eq!(usage.spent_lifetime_lamports, 20_600);
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn durable_action_identity_is_idempotent_across_repetition_and_restart(
        amount in 1_u64..=1_000_000_000,
        repetitions in 1_usize..16,
    ) {
        let (_directory, path, store) = fixture()
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        let now = instant(17, 11, 0)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        let action = native_action(
            RunId(Uuid::from_u128(901)),
            AgentId(Uuid::from_u128(902)),
            0,
            now,
            amount,
        );
        let inserted = store
            .enqueue_action(&action)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        prop_assert!(inserted);
        for _ in 1..repetitions {
            let replayed = store
                .enqueue_action(&action)
                .map_err(|error| TestCaseError::fail(error.to_string()))?;
            prop_assert!(!replayed);
        }
        let events = store
            .action_events(&action.id)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        prop_assert_eq!(events.len(), 1);
        drop(store);

        let identity = StoreIdentity::surfpool("test-surfnet")
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        let reopened = Store::open(&path, identity)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        let replayed = reopened
            .enqueue_action(&action)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        prop_assert!(!replayed);
        let mut conflict = action.clone();
        conflict.payload = ActionPayload::NativeTransfer {
            destination: "destination".to_owned(),
            lamports: amount + 1,
        };
        prop_assert!(reopened.enqueue_action(&conflict).is_err());
        let events = reopened
            .action_events(&action.id)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        prop_assert_eq!(events.len(), 1);
    }
}
