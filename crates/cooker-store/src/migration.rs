use std::{collections::BTreeMap, fmt, time::Duration};

use chrono::{SecondsFormat, Utc};
use cooker_core::CookerError;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use uuid::Uuid;

use crate::StoreIdentity;

pub(crate) const APPLICATION_ID: i64 = 0x434f_4f4b;
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

struct Migration {
    version: u32,
    name: &'static str,
    sql: &'static str,
}

const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "initial",
        sql: include_str!("../migrations/0001_initial.sql"),
    },
    Migration {
        version: 2,
        name: "immutable_journals",
        sql: include_str!("../migrations/0002_immutable_journals.sql"),
    },
    Migration {
        version: 3,
        name: "account_creation_budget",
        sql: include_str!("../migrations/0003_account_creation_budget.sql"),
    },
    Migration {
        version: 4,
        name: "action_deferrals",
        sql: include_str!("../migrations/0004_action_deferrals.sql"),
    },
    Migration {
        version: 5,
        name: "agent_planner_cursor",
        sql: include_str!("../migrations/0005_agent_planner_cursor.sql"),
    },
    Migration {
        version: 6,
        name: "confirmation_audits",
        sql: include_str!("../migrations/0006_confirmation_audits.sql"),
    },
];

pub(crate) fn configure(connection: &Connection) -> Result<(), CookerError> {
    connection
        .busy_timeout(BUSY_TIMEOUT)
        .map_err(|error| sqlite_error("configure busy timeout", error))?;
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .map_err(|error| sqlite_error("enable foreign keys", error))?;
    connection
        .pragma_update(None, "synchronous", "FULL")
        .map_err(|error| sqlite_error("enable full synchronization", error))?;
    connection
        .pragma_update(None, "trusted_schema", "OFF")
        .map_err(|error| sqlite_error("disable trusted schema", error))?;
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .map_err(|error| sqlite_error("select WAL journal mode", error))?;
    let journal_mode: String = connection
        .pragma_query_value(None, "journal_mode", |row| row.get(0))
        .map_err(|error| sqlite_error("read journal mode", error))?;
    if !journal_mode.eq_ignore_ascii_case("wal") {
        return Err(CookerError::Store(format!(
            "SQLite refused WAL journal mode and selected {journal_mode}"
        )));
    }
    Ok(())
}

pub(crate) fn configure_read_only(
    connection: &Connection,
    require_wal_mode: bool,
) -> Result<(), CookerError> {
    connection
        .busy_timeout(BUSY_TIMEOUT)
        .map_err(|error| sqlite_error("configure read-only busy timeout", error))?;
    let journal_mode: String = connection
        .pragma_query_value(None, "journal_mode", |row| row.get(0))
        .map_err(|error| sqlite_error("read journal mode", error))?;
    if require_wal_mode && !journal_mode.eq_ignore_ascii_case("wal") {
        return Err(CookerError::Store(format!(
            "read-only store is not in WAL mode: {journal_mode}"
        )));
    }
    Ok(())
}

pub(crate) fn verify_current(
    connection: &Connection,
    identity: &StoreIdentity,
) -> Result<Uuid, CookerError> {
    identity.validate()?;
    let application_id: i64 = connection
        .pragma_query_value(None, "application_id", |row| row.get(0))
        .map_err(|error| sqlite_error("verify read-only SQLite application id", error))?;
    if application_id != APPLICATION_ID {
        return Err(CookerError::Store(format!(
            "SQLite application id mismatch: expected {APPLICATION_ID}, found {application_id}"
        )));
    }
    let expected_version = MIGRATIONS.last().map_or(0, |migration| migration.version);
    let user_version: u32 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|error| sqlite_error("read schema version", error))?;
    if user_version != expected_version {
        return Err(CookerError::Store(format!(
            "database schema version {user_version} is not current version {expected_version}"
        )));
    }

    let applied = read_migration_journal(connection)?;
    if applied.len() != MIGRATIONS.len() {
        return Err(CookerError::Store(
            "database migration journal is incomplete or contains unknown entries".to_owned(),
        ));
    }
    for migration in MIGRATIONS {
        let expected_checksum = blake3::hash(migration.sql.as_bytes()).to_hex().to_string();
        let Some((name, checksum)) = applied.get(&migration.version) else {
            return Err(CookerError::Store(format!(
                "database is missing migration {}",
                migration.version
            )));
        };
        if name != migration.name || checksum != &expected_checksum {
            return Err(CookerError::Store(format!(
                "migration {} integrity mismatch",
                migration.version
            )));
        }
    }

    let (database_id, network, surfnet_id): (String, String, String) = connection
        .query_row(
            "SELECT database_id, network, surfnet_id FROM store_identity WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|error| sqlite_error("load read-only store identity", error))?;
    if network != identity.network || surfnet_id != identity.surfnet_id {
        return Err(CookerError::InvalidConfig(format!(
            "database identity mismatch: expected network={} surfnet_id={}, found network={network} surfnet_id={surfnet_id}",
            identity.network, identity.surfnet_id
        )));
    }
    let database_id = Uuid::parse_str(&database_id).map_err(|error| {
        CookerError::Store(format!("invalid persisted database identity: {error}"))
    })?;
    verify(connection)?;
    Ok(database_id)
}

#[allow(clippy::too_many_lines)]
pub(crate) fn initialize(
    connection: &mut Connection,
    identity: &StoreIdentity,
) -> Result<Uuid, CookerError> {
    identity.validate()?;
    let application_id: i64 = connection
        .pragma_query_value(None, "application_id", |row| row.get(0))
        .map_err(|error| sqlite_error("read SQLite application id", error))?;
    let user_version: u32 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|error| sqlite_error("read schema version", error))?;
    let latest = MIGRATIONS.last().map_or(0, |migration| migration.version);
    if application_id == APPLICATION_ID && user_version == latest {
        return verify_current(connection, identity);
    }
    establish_application_identity(connection)?;

    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| sqlite_error("begin migration transaction", error))?;
    transaction
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (\
                 version INTEGER PRIMARY KEY CHECK (version > 0),\
                 name TEXT NOT NULL UNIQUE,\
                 checksum TEXT NOT NULL,\
                 applied_at TEXT NOT NULL\
             );",
        )
        .map_err(|error| sqlite_error("create migration journal", error))?;

    let applied = read_migration_journal(&transaction)?;

    if applied.keys().any(|version| {
        MIGRATIONS
            .last()
            .is_none_or(|latest| *version > latest.version)
    }) {
        return Err(CookerError::Store(
            "database schema is newer than this binary".to_owned(),
        ));
    }

    for migration in MIGRATIONS {
        let checksum = blake3::hash(migration.sql.as_bytes()).to_hex().to_string();
        if let Some((stored_name, stored_checksum)) = applied.get(&migration.version) {
            if stored_name != migration.name || stored_checksum != &checksum {
                return Err(CookerError::Store(format!(
                    "migration {} integrity mismatch",
                    migration.version
                )));
            }
            continue;
        }
        transaction
            .execute_batch(migration.sql)
            .map_err(|error| sqlite_error("apply embedded migration", error))?;
        transaction
            .execute(
                "INSERT INTO schema_migrations(version, name, checksum, applied_at) \
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    migration.version,
                    migration.name,
                    checksum,
                    timestamp(Utc::now())
                ],
            )
            .map_err(|error| sqlite_error("journal embedded migration", error))?;
    }

    transaction
        .pragma_update(None, "user_version", latest)
        .map_err(|error| sqlite_error("set schema version", error))?;

    let stored_identity: Option<(String, String, String)> = transaction
        .query_row(
            "SELECT database_id, network, surfnet_id FROM store_identity WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|error| sqlite_error("load store identity", error))?;

    let database_id = if let Some((database_id, network, surfnet_id)) = stored_identity {
        if network != identity.network || surfnet_id != identity.surfnet_id {
            return Err(CookerError::InvalidConfig(format!(
                "database identity mismatch: expected network={} surfnet_id={}, found network={network} surfnet_id={surfnet_id}",
                identity.network, identity.surfnet_id
            )));
        }
        Uuid::parse_str(&database_id).map_err(|error| {
            CookerError::Store(format!("invalid persisted database identity: {error}"))
        })?
    } else {
        let database_id = Uuid::new_v4();
        transaction
            .execute(
                "INSERT INTO store_identity(\
                    singleton, database_id, network, surfnet_id, created_at\
                 ) VALUES (1, ?1, ?2, ?3, ?4)",
                params![
                    database_id.to_string(),
                    identity.network,
                    identity.surfnet_id,
                    timestamp(Utc::now())
                ],
            )
            .map_err(|error| sqlite_error("persist store identity", error))?;
        database_id
    };

    transaction
        .commit()
        .map_err(|error| sqlite_error("commit migrations", error))?;
    verify(connection)?;
    Ok(database_id)
}

/// Read the recorded migration journal as version to (name, checksum).
fn read_migration_journal(
    connection: &Connection,
) -> Result<BTreeMap<u32, (String, String)>, CookerError> {
    let mut applied = BTreeMap::new();
    let mut statement = connection
        .prepare("SELECT version, name, checksum FROM schema_migrations ORDER BY version")
        .map_err(|error| sqlite_error("prepare migration journal query", error))?;
    let mut rows = statement
        .query([])
        .map_err(|error| sqlite_error("query migration journal", error))?;
    while let Some(row) = rows
        .next()
        .map_err(|error| sqlite_error("read migration journal", error))?
    {
        let version: u32 = row
            .get(0)
            .map_err(|error| sqlite_error("decode migration version", error))?;
        let name: String = row
            .get(1)
            .map_err(|error| sqlite_error("decode migration name", error))?;
        let checksum: String = row
            .get(2)
            .map_err(|error| sqlite_error("decode migration checksum", error))?;
        applied.insert(version, (name, checksum));
    }
    Ok(applied)
}

fn establish_application_identity(connection: &Connection) -> Result<(), CookerError> {
    let current: i64 = connection
        .pragma_query_value(None, "application_id", |row| row.get(0))
        .map_err(|error| sqlite_error("read SQLite application id", error))?;
    if current == APPLICATION_ID {
        return Ok(());
    }
    if current != 0 {
        return Err(CookerError::Store(format!(
            "SQLite application id mismatch: expected {APPLICATION_ID}, found {current}"
        )));
    }
    let table_count: i64 = connection
        .query_row(
            "SELECT count(*) FROM sqlite_schema \
             WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| sqlite_error("inspect unidentified database", error))?;
    if table_count != 0 {
        return Err(CookerError::Store(
            "refusing to adopt a non-empty unidentified SQLite database".to_owned(),
        ));
    }
    connection
        .pragma_update(None, "application_id", APPLICATION_ID)
        .map_err(|error| sqlite_error("set SQLite application id", error))?;
    Ok(())
}

pub(crate) fn verify(connection: &Connection) -> Result<(), CookerError> {
    let application_id: i64 = connection
        .pragma_query_value(None, "application_id", |row| row.get(0))
        .map_err(|error| sqlite_error("verify SQLite application id", error))?;
    if application_id != APPLICATION_ID {
        return Err(CookerError::Store(format!(
            "SQLite application id changed to {application_id}"
        )));
    }
    let result: String = connection
        .query_row("PRAGMA quick_check(1)", [], |row| row.get(0))
        .map_err(|error| sqlite_error("run SQLite quick check", error))?;
    if result != "ok" {
        return Err(CookerError::Store(format!(
            "SQLite quick check failed: {result}"
        )));
    }
    let mut statement = connection
        .prepare("PRAGMA foreign_key_check")
        .map_err(|error| sqlite_error("prepare foreign-key check", error))?;
    let mut rows = statement
        .query([])
        .map_err(|error| sqlite_error("run foreign-key check", error))?;
    if rows
        .next()
        .map_err(|error| sqlite_error("read foreign-key check", error))?
        .is_some()
    {
        return Err(CookerError::Store(
            "SQLite foreign-key check failed".to_owned(),
        ));
    }
    Ok(())
}

pub(crate) fn timestamp(value: chrono::DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Nanos, true)
}

pub(crate) fn sqlite_error(context: &str, error: impl fmt::Display) -> CookerError {
    CookerError::Store(format!("{context}: {error}"))
}
