//! Surfpool-only bounded funding, execution, and recovery commands.

use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    io::Write,
    path::{Path, PathBuf},
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, TimeDelta, Utc};
use cooker_core::{
    ActionAdapter, ActionId, ActionKind, ActionPayload, AdapterContext, AgentId, AgentSnapshot,
    BudgetConfig, ChainGateway, Clock, CookerError, PersonaBehaviorModel, PlannedAction, Policy,
    PolicyConfig, RunId, SafetyPolicy, SessionState, StateStore, SystemClock,
};
use cooker_runtime::{
    FleetRuntime, NoFaults, PlannerFleet, PlanningSummary, RuntimeEngine, RuntimeSettings,
    WorkerSummary,
};
use cooker_solana::{
    FleetManifest, JupiterAdapter, JupiterApiClient, JupiterPolicy, LocalKeypair,
    NativeStakeAdapter, NativeTransferAdapter, RpcEndpoint, SolanaGateway, SplTransferAdapter,
};
use cooker_store::{RecoveryPreview, RunRegistration, Store, StoreIdentity, StoreStatusSnapshot};
use serde::Serialize;
use solana_pubkey::Pubkey;
use tokio::{sync::watch, task::JoinHandle, time::MissedTickBehavior};
use uuid::Uuid;

use crate::config::{FileConfig, JupiterFileConfig, parse_seed};

const MODEL_VERSION: &str = "behavior-v1";
const FUNDING_MODEL_VERSION: &str = "surfpool-funding-v1";
const WRAPPED_SOL_MINT: &str = "So11111111111111111111111111111111111111112";
const DESTINATIONS_PER_AGENT: usize = 8;
const STOP_NONE: u8 = 0;
const STOP_CTRL_C: u8 = 1;
const STOP_FILE: u8 = 2;
const STOP_MONITOR_ERROR: u8 = 3;

#[derive(Debug)]
struct OperatorStopMonitor {
    reason: Arc<AtomicU8>,
    task: JoinHandle<()>,
}

impl OperatorStopMonitor {
    fn reason(&self) -> Option<&'static str> {
        stop_reason(self.reason.load(Ordering::SeqCst))
    }
}

impl Drop for OperatorStopMonitor {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Clone, Debug, Serialize)]
struct PlanningReport {
    decisions: usize,
    actions_enqueued: usize,
    actions_replayed: usize,
    idle_decisions: usize,
    conflicts: usize,
}

impl From<PlanningSummary> for PlanningReport {
    fn from(value: PlanningSummary) -> Self {
        Self {
            decisions: value.decisions,
            actions_enqueued: value.actions_enqueued,
            actions_replayed: value.actions_replayed,
            idle_decisions: value.idle_decisions,
            conflicts: value.conflicts,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
struct WorkerReport {
    confirmed: usize,
    audited: usize,
    orphaned: usize,
    rejected: usize,
    delayed: usize,
    unknown: usize,
    expired: usize,
    failed: usize,
    errors: Vec<String>,
    peak_workers: usize,
    released_without_start: usize,
}

impl From<WorkerSummary> for WorkerReport {
    fn from(value: WorkerSummary) -> Self {
        Self {
            confirmed: value.confirmed,
            audited: value.audited,
            orphaned: value.orphaned,
            rejected: value.rejected,
            delayed: value.delayed,
            unknown: value.unknown,
            expired: value.expired,
            failed: value.failed,
            errors: value.errors,
            peak_workers: value.peak_workers,
            released_without_start: value.released_without_start,
        }
    }
}

#[derive(Debug, Serialize)]
struct RunReport {
    schema_version: u16,
    mode: &'static str,
    database: String,
    surfnet_id: String,
    run_id: RunId,
    agents: usize,
    enabled_actions: BTreeSet<ActionKind>,
    planning_limit: usize,
    action_limit: usize,
    planning: Option<PlanningReport>,
    claimed_actions: usize,
    workers: WorkerReport,
    before: StoreStatusSnapshot,
    after: StoreStatusSnapshot,
    network_preflight: bool,
    signer_loaded: bool,
    stop_reason: Option<&'static str>,
    state_changed: bool,
}

#[derive(Debug, Serialize)]
struct FundingReport {
    schema_version: u16,
    mode: &'static str,
    database: String,
    surfnet_id: String,
    fleet_run_id: RunId,
    funding_run_id: RunId,
    fleet_agents: usize,
    lamports_per_agent: u64,
    action_limit: usize,
    actions_inserted: usize,
    claimed_actions: usize,
    recovery_claimed: usize,
    recovery: WorkerReport,
    workers: WorkerReport,
    snapshot: Option<StoreStatusSnapshot>,
    network_preflight: bool,
    signer_loaded: bool,
    stop_reason: Option<&'static str>,
    state_changed: bool,
}

#[derive(Debug, Serialize)]
struct RecoveryReport {
    schema_version: u16,
    mode: &'static str,
    database: String,
    surfnet_id: String,
    run_id: RunId,
    limit: usize,
    released_expired_leases: usize,
    reconciliation_claimed: usize,
    audit_claimed: usize,
    reconciliation: WorkerReport,
    audits: WorkerReport,
    preview_before: RecoveryPreview,
    preview_after: RecoveryPreview,
    transaction_submissions: u8,
    network_preflight: bool,
    signer_loaded: bool,
    stop_reason: Option<&'static str>,
    state_changed: bool,
}

pub(crate) fn run(
    config_path: &Path,
    project_root: &Path,
    database: &Path,
    limit: usize,
    planning_limit: usize,
    execute: bool,
    output: &mut impl Write,
) -> Result<()> {
    if limit == 0 || limit > 10_000 || planning_limit == 0 || planning_limit > 1_000_000 {
        bail!("run requires limit in 1..=10000 and planning-limit in 1..=1000000");
    }
    let file = load_validated(config_path)?;
    let root = canonical_root(project_root)?;
    ensure_operator_stop_clear(&file, &root)?;
    let database = resolve_path(&root, database);
    require_database(&database)?;
    if execute {
        execute_run(&file, &root, &database, limit, planning_limit, output)
    } else {
        preview_run(&file, &root, &database, limit, planning_limit, output)
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn fund(
    config_path: &Path,
    project_root: &Path,
    database: &Path,
    funder_path: &Path,
    lamports_per_agent: Option<u64>,
    limit: usize,
    execute: bool,
    output: &mut impl Write,
) -> Result<()> {
    if limit == 0 || limit > 10_000 {
        bail!("fund requires limit in 1..=10000");
    }
    let file = load_validated(config_path)?;
    let root = canonical_root(project_root)?;
    ensure_operator_stop_clear(&file, &root)?;
    let database = resolve_path(&root, database);
    let funder_path = resolve_path(&root, funder_path);
    let amount = lamports_per_agent.unwrap_or(file.online.funding_lamports_per_agent);
    if amount == 0 {
        bail!("fund requires a positive lamports-per-agent value");
    }
    if execute {
        execute_funding(&file, &root, &database, &funder_path, amount, limit, output)
    } else {
        preview_funding(&file, &root, &database, amount, limit, output)
    }
}

pub(crate) fn recover(
    config_path: &Path,
    project_root: &Path,
    database: &Path,
    limit: usize,
    audit_age_seconds: Option<u64>,
    execute: bool,
    output: &mut impl Write,
) -> Result<()> {
    if limit == 0 || limit > 10_000 {
        bail!("recover requires limit in 1..=10000");
    }
    let file = load_validated(config_path)?;
    let root = canonical_root(project_root)?;
    ensure_operator_stop_clear(&file, &root)?;
    let database = resolve_path(&root, database);
    require_database(&database)?;
    let audit_age = audit_age_seconds.unwrap_or(file.online.confirmation_audit_age_seconds);
    if execute {
        execute_recovery(&file, &root, &database, limit, audit_age, output)
    } else {
        preview_recovery(&file, &root, &database, limit, audit_age, output)
    }
}

fn preview_run(
    file: &FileConfig,
    root: &Path,
    database: &Path,
    limit: usize,
    planning_limit: usize,
    output: &mut impl Write,
) -> Result<()> {
    let manifest = load_public_manifest(file, root)?;
    let store = open_read_only_store(file, database)?;
    verify_execution_run(file, &manifest, &store)?;
    let now = Utc::now();
    let before = store
        .status_snapshot(now, None)
        .map_err(anyhow::Error::msg)?;
    let report = RunReport {
        schema_version: 1,
        mode: "preview",
        database: database.display().to_string(),
        surfnet_id: file.core.network.surfnet_id.clone(),
        run_id: manifest.run_id(),
        agents: manifest.entries().len(),
        enabled_actions: file.online.enabled_actions.clone(),
        planning_limit,
        action_limit: limit,
        planning: None,
        claimed_actions: 0,
        workers: WorkerReport::default(),
        before: before.clone(),
        after: before,
        network_preflight: false,
        signer_loaded: false,
        stop_reason: None,
        state_changed: false,
    };
    write_json(output, &report)
}

fn execute_run(
    file: &FileConfig,
    root: &Path,
    database: &Path,
    limit: usize,
    planning_limit: usize,
    output: &mut impl Write,
) -> Result<()> {
    let runtime = async_runtime("run")?;
    runtime.block_on(async {
        let (stop_monitor, stop) =
            spawn_operator_stop_monitor(kill_switch_path(file, root));
        let gateway = connect_surfpool(file).await?;
        fail_if_operator_stopped(&stop_monitor)?;
        let manifest = load_private_manifest(file, root)?;
        let store = open_writable_store(file, database)?;
        verify_execution_run(file, &manifest, &store)?;
        let now = Utc::now();
        let before = store.status_snapshot(now, None).map_err(anyhow::Error::msg)?;
        if before.unresolved_actions != 0 {
            bail!(
                "runtime has {} unresolved action(s); run recover --execute before claiming new work",
                before.unresolved_actions
            );
        }
        let clock: Arc<dyn Clock> = Arc::new(SystemClock);
        let fleet = build_fleet_runtime(
            file,
            root,
            &manifest,
            Arc::clone(&store),
            &gateway,
            Arc::clone(&clock),
        )?;
        let planner = build_planner(file, &manifest, Arc::clone(&store), Arc::clone(&clock))?;
        let released_expired_leases = store
            .release_expired_leases(now)
            .map_err(anyhow::Error::msg)?;
        let planning = plan_until_limit(&planner, manifest.entries().len(), planning_limit)?;
        let claimed_at = clock.now();
        let leases = if *stop.borrow() {
            Vec::new()
        } else {
            store
                .claim_due_actions(
                    &worker_id("run"),
                    claimed_at,
                    lease_until(file, claimed_at)?,
                    limit,
                )
                .map_err(anyhow::Error::msg)?
        };
        let claimed_actions = leases.len();
        let workers: WorkerReport = fleet.run_claimed(leases, stop.clone()).await.into();
        let after = store
            .status_snapshot(clock.now(), None)
            .map_err(anyhow::Error::msg)?;
        let has_errors = !workers.errors.is_empty();
        let stop_reason = stop_monitor.reason();
        let state_changed = released_expired_leases != 0
            || planning.decisions != 0
            || claimed_actions != 0;
        let report = RunReport {
            schema_version: 1,
            mode: "execute",
            database: database.display().to_string(),
            surfnet_id: file.core.network.surfnet_id.clone(),
            run_id: manifest.run_id(),
            agents: manifest.entries().len(),
            enabled_actions: file.online.enabled_actions.clone(),
            planning_limit,
            action_limit: limit,
            planning: Some(planning.into()),
            claimed_actions,
            workers,
            before,
            after,
            network_preflight: true,
            signer_loaded: true,
            stop_reason,
            state_changed,
        };
        write_json(output, &report)?;
        if let Some(reason) = stop_reason {
            bail!("operator stop activated by {reason}; in-flight work settled");
        }
        if has_errors {
            bail!("one or more runtime workers failed; inspect the JSON report and run recovery");
        }
        Ok(())
    })
}

fn preview_funding(
    file: &FileConfig,
    root: &Path,
    database: &Path,
    amount: u64,
    limit: usize,
    output: &mut impl Write,
) -> Result<()> {
    let manifest = load_public_manifest(file, root)?;
    let funding_run_id = funding_run_id(manifest.run_id());
    let snapshot = if database.is_file() {
        Some(
            open_read_only_store(file, database)?
                .status_snapshot(Utc::now(), None)
                .map_err(anyhow::Error::msg)?,
        )
    } else {
        None
    };
    let report = FundingReport {
        schema_version: 1,
        mode: "preview",
        database: database.display().to_string(),
        surfnet_id: file.core.network.surfnet_id.clone(),
        fleet_run_id: manifest.run_id(),
        funding_run_id,
        fleet_agents: manifest.entries().len(),
        lamports_per_agent: amount,
        action_limit: limit,
        actions_inserted: 0,
        claimed_actions: 0,
        recovery_claimed: 0,
        recovery: WorkerReport::default(),
        workers: WorkerReport::default(),
        snapshot,
        network_preflight: false,
        signer_loaded: false,
        stop_reason: None,
        state_changed: false,
    };
    write_json(output, &report)
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn execute_funding(
    file: &FileConfig,
    root: &Path,
    database: &Path,
    funder_path: &Path,
    amount: u64,
    limit: usize,
    output: &mut impl Write,
) -> Result<()> {
    let runtime = async_runtime("fund")?;
    runtime.block_on(async {
        let (stop_monitor, stop) = spawn_operator_stop_monitor(kill_switch_path(file, root));
        let gateway = connect_surfpool(file).await?;
        fail_if_operator_stopped(&stop_monitor)?;
        let manifest = load_private_manifest(file, root)?;
        if let Some(parent) = database.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create funding database directory {}",
                    parent.display()
                )
            })?;
        }
        let store = open_writable_store(file, database)?;
        let funder = Arc::new(
            LocalKeypair::load(root, funder_path)
                .map_err(anyhow::Error::msg)
                .context("failed to load Surfpool funder after network preflight")?,
        );
        let now = Utc::now();
        let funding_epoch = manifest.created_at();
        let funding_run_id = funding_run_id(manifest.run_id());
        let funding_agent_id = AgentId::derive(funding_run_id, 0);
        let clock: Arc<dyn Clock> = Arc::new(SystemClock);
        let fleet = build_funding_runtime(
            file,
            &manifest,
            &store,
            &gateway,
            &funder,
            funding_agent_id,
            amount,
            Arc::clone(&clock),
        )?;
        fail_if_operator_stopped(&stop_monitor)?;
        let registration_changed = register_funding_run(
            &store,
            funding_run_id,
            funding_agent_id,
            manifest.run_id(),
            amount,
            manifest.entries().len(),
            funding_epoch,
        )?;

        store
            .release_expired_leases(now)
            .map_err(anyhow::Error::msg)?;
        let recovery_leases = store
            .claim_reconciliation_candidates(
                &worker_id("fund-recover"),
                now,
                lease_until(file, now)?,
                limit,
            )
            .map_err(anyhow::Error::msg)?;
        let recovery_claimed = recovery_leases.len();
        let recovery: WorkerReport = Arc::clone(&fleet)
            .reconcile_claimed(recovery_leases, stop.clone())
            .await
            .into();
        if !recovery.errors.is_empty() {
            let snapshot = store
                .status_snapshot(clock.now(), None)
                .map_err(anyhow::Error::msg)?;
            let report = FundingReport {
                schema_version: 1,
                mode: "execute",
                database: database.display().to_string(),
                surfnet_id: file.core.network.surfnet_id.clone(),
                fleet_run_id: manifest.run_id(),
                funding_run_id,
                fleet_agents: manifest.entries().len(),
                lamports_per_agent: amount,
                action_limit: limit,
                actions_inserted: 0,
                claimed_actions: 0,
                recovery_claimed,
                recovery,
                workers: WorkerReport::default(),
                snapshot: Some(snapshot),
                network_preflight: true,
                signer_loaded: true,
                stop_reason: stop_monitor.reason(),
                state_changed: registration_changed || recovery_claimed != 0,
            };
            write_json(output, &report)?;
            bail!("funding reconciliation failed; no new funding actions were claimed");
        }
        let remaining = store
            .recovery_preview(clock.now(), None, 0)
            .map_err(anyhow::Error::msg)?;
        if remaining.counts.reconciliation_candidates != 0 {
            bail!("funding recovery remains incomplete; repeat the bounded fund command");
        }

        let mut actions_inserted = 0;
        if !*stop.borrow() {
            for entry in manifest.entries() {
                let action = funding_action(
                    funding_run_id,
                    funding_agent_id,
                    entry.index,
                    &entry.public_key,
                    amount,
                    file.core.budgets.max_fee_lamports,
                    funding_epoch,
                );
                actions_inserted +=
                    usize::from(store.enqueue_action(&action).map_err(anyhow::Error::msg)?);
            }
        }
        let claimed_at = clock.now();
        let leases = if *stop.borrow() {
            Vec::new()
        } else {
            store
                .claim_due_actions(
                    &worker_id("fund"),
                    claimed_at,
                    lease_until(file, claimed_at)?,
                    limit,
                )
                .map_err(anyhow::Error::msg)?
        };
        let claimed_actions = leases.len();
        let workers: WorkerReport = fleet.run_claimed(leases, stop.clone()).await.into();
        let snapshot = store
            .status_snapshot(clock.now(), None)
            .map_err(anyhow::Error::msg)?;
        let has_errors = !workers.errors.is_empty();
        let stop_reason = stop_monitor.reason();
        let report = FundingReport {
            schema_version: 1,
            mode: "execute",
            database: database.display().to_string(),
            surfnet_id: file.core.network.surfnet_id.clone(),
            fleet_run_id: manifest.run_id(),
            funding_run_id,
            fleet_agents: manifest.entries().len(),
            lamports_per_agent: amount,
            action_limit: limit,
            actions_inserted,
            claimed_actions,
            recovery_claimed,
            recovery,
            workers,
            snapshot: Some(snapshot),
            network_preflight: true,
            signer_loaded: true,
            stop_reason,
            state_changed: registration_changed
                || recovery_claimed != 0
                || actions_inserted != 0
                || claimed_actions != 0,
        };
        write_json(output, &report)?;
        if let Some(reason) = stop_reason {
            bail!("operator stop activated by {reason}; in-flight funding work settled");
        }
        if has_errors {
            bail!("one or more funding workers failed; rerun fund --execute to reconcile first");
        }
        Ok(())
    })
}

fn preview_recovery(
    file: &FileConfig,
    root: &Path,
    database: &Path,
    limit: usize,
    audit_age_seconds: u64,
    output: &mut impl Write,
) -> Result<()> {
    let manifest = load_public_manifest(file, root)?;
    let store = open_read_only_store(file, database)?;
    verify_execution_run(file, &manifest, &store)?;
    let now = Utc::now();
    let cutoff = audit_cutoff(now, audit_age_seconds)?;
    let preview = store
        .recovery_preview(now, Some(cutoff), limit)
        .map_err(anyhow::Error::msg)?;
    let report = RecoveryReport {
        schema_version: 1,
        mode: "preview",
        database: database.display().to_string(),
        surfnet_id: file.core.network.surfnet_id.clone(),
        run_id: manifest.run_id(),
        limit,
        released_expired_leases: 0,
        reconciliation_claimed: 0,
        audit_claimed: 0,
        reconciliation: WorkerReport::default(),
        audits: WorkerReport::default(),
        preview_before: preview.clone(),
        preview_after: preview,
        transaction_submissions: 0,
        network_preflight: false,
        signer_loaded: false,
        stop_reason: None,
        state_changed: false,
    };
    write_json(output, &report)
}

#[allow(clippy::too_many_lines)]
fn execute_recovery(
    file: &FileConfig,
    root: &Path,
    database: &Path,
    limit: usize,
    audit_age_seconds: u64,
    output: &mut impl Write,
) -> Result<()> {
    let runtime = async_runtime("recover")?;
    runtime.block_on(async {
        let (stop_monitor, stop) = spawn_operator_stop_monitor(kill_switch_path(file, root));
        let gateway = connect_surfpool(file).await?;
        fail_if_operator_stopped(&stop_monitor)?;
        let manifest = load_private_manifest(file, root)?;
        let store = open_writable_store(file, database)?;
        verify_execution_run(file, &manifest, &store)?;
        let clock: Arc<dyn Clock> = Arc::new(SystemClock);
        let now = clock.now();
        let cutoff = audit_cutoff(now, audit_age_seconds)?;
        let preview_before = store
            .recovery_preview(now, Some(cutoff), limit)
            .map_err(anyhow::Error::msg)?;
        let fleet = build_fleet_runtime(
            file,
            root,
            &manifest,
            Arc::clone(&store),
            &gateway,
            Arc::clone(&clock),
        )?;
        let released_expired_leases = store
            .release_expired_leases(now)
            .map_err(anyhow::Error::msg)?;
        let reconcile_at = clock.now();
        let reconciliation_leases = if *stop.borrow() {
            Vec::new()
        } else {
            store
                .claim_reconciliation_candidates(
                    &worker_id("recover"),
                    reconcile_at,
                    lease_until(file, reconcile_at)?,
                    limit,
                )
                .map_err(anyhow::Error::msg)?
        };
        let reconciliation_claimed = reconciliation_leases.len();
        let reconciliation: WorkerReport = Arc::clone(&fleet)
            .reconcile_claimed(reconciliation_leases, stop.clone())
            .await
            .into();

        let audit_at = clock.now();
        let audit_leases = if *stop.borrow() {
            Vec::new()
        } else {
            store
                .claim_confirmation_audit_candidates(
                    &worker_id("audit"),
                    audit_at,
                    lease_until(file, audit_at)?,
                    cutoff,
                    limit,
                )
                .map_err(anyhow::Error::msg)?
        };
        let audit_claimed = audit_leases.len();
        let audits: WorkerReport = fleet.audit_claimed(audit_leases, stop.clone()).await.into();
        let preview_after = store
            .recovery_preview(clock.now(), Some(cutoff), limit)
            .map_err(anyhow::Error::msg)?;
        let has_errors = !reconciliation.errors.is_empty() || !audits.errors.is_empty();
        let stop_reason = stop_monitor.reason();
        let report = RecoveryReport {
            schema_version: 1,
            mode: "execute",
            database: database.display().to_string(),
            surfnet_id: file.core.network.surfnet_id.clone(),
            run_id: manifest.run_id(),
            limit,
            released_expired_leases,
            reconciliation_claimed,
            audit_claimed,
            reconciliation,
            audits,
            preview_before,
            preview_after,
            transaction_submissions: 0,
            network_preflight: true,
            signer_loaded: true,
            stop_reason,
            state_changed: released_expired_leases != 0
                || reconciliation_claimed != 0
                || audit_claimed != 0,
        };
        write_json(output, &report)?;
        if let Some(reason) = stop_reason {
            bail!("operator stop activated by {reason}; in-flight recovery work settled");
        }
        if has_errors {
            bail!("one or more recovery workers failed; inspect the JSON report");
        }
        Ok(())
    })
}

fn build_planner(
    file: &FileConfig,
    manifest: &FleetManifest,
    store: Arc<Store>,
    clock: Arc<dyn Clock>,
) -> Result<PlannerFleet> {
    let seed = parse_seed(&file.core.seed_hex)?;
    let personas = file.offline.persona_configs();
    let mut models = BTreeMap::new();
    for entry in manifest.entries() {
        let persona_index = usize::try_from(entry.index)
            .context("fleet index does not fit usize")?
            % personas.len();
        let catalog = execution_catalog(file, manifest, entry.index)?;
        let model = PersonaBehaviorModel::new(
            seed,
            MODEL_VERSION,
            personas[persona_index].clone(),
            catalog,
            file.core.budgets.max_fee_lamports,
            file.core.budgets.max_account_creation_lamports,
            file.offline.max_slippage_bps,
        )
        .map_err(anyhow::Error::msg)?;
        models.insert(entry.agent_id, Arc::new(model));
    }
    let store: Arc<dyn StateStore> = store;
    PlannerFleet::new(
        store,
        models,
        clock,
        manifest.run_id(),
        seed,
        file.core.budgets.daily_lamports,
    )
    .map_err(anyhow::Error::msg)
}

fn plan_until_limit(
    planner: &PlannerFleet,
    fleet_size: usize,
    planning_limit: usize,
) -> Result<PlanningSummary> {
    let mut aggregate = PlanningSummary::default();
    while aggregate.decisions < planning_limit {
        let remaining = planning_limit - aggregate.decisions;
        let pass = planner
            .plan_due(remaining.min(fleet_size))
            .map_err(anyhow::Error::msg)?;
        aggregate.actions_enqueued += pass.actions_enqueued;
        aggregate.actions_replayed += pass.actions_replayed;
        aggregate.idle_decisions += pass.idle_decisions;
        aggregate.conflicts += pass.conflicts;
        if pass.decisions == 0 {
            break;
        }
        aggregate.decisions += pass.decisions;
    }
    Ok(aggregate)
}

fn execution_catalog(
    file: &FileConfig,
    manifest: &FleetManifest,
    own_index: u64,
) -> Result<cooker_core::ActionCatalog> {
    let entries = manifest.entries();
    let own = usize::try_from(own_index).context("fleet index does not fit usize")?;
    let mut native_destinations = Vec::new();
    if file
        .online
        .enabled_actions
        .contains(&ActionKind::NativeTransfer)
        && entries.len() > 1
    {
        let count = DESTINATIONS_PER_AGENT.min(entries.len() - 1);
        for offset in 1..=count {
            native_destinations.push(entries[(own + offset) % entries.len()].public_key.clone());
        }
    }
    Ok(cooker_core::ActionCatalog {
        native_destinations,
        spl_routes: if file
            .online
            .enabled_actions
            .contains(&ActionKind::SplTransfer)
        {
            file.online.spl_routes.clone()
        } else {
            Vec::new()
        },
        swap_routes: if file
            .online
            .enabled_actions
            .contains(&ActionKind::JupiterSwap)
        {
            file.online.swap_routes.clone()
        } else {
            Vec::new()
        },
        allow_stake_enter: file
            .online
            .enabled_actions
            .contains(&ActionKind::StakeLifecycle),
    })
}

fn build_fleet_runtime(
    file: &FileConfig,
    root: &Path,
    manifest: &FleetManifest,
    store: Arc<Store>,
    gateway: &Arc<SolanaGateway>,
    clock: Arc<dyn Clock>,
) -> Result<Arc<FleetRuntime>> {
    let signers = manifest
        .load_signers(root)
        .map_err(anyhow::Error::msg)
        .context("failed to load fleet signers after Surfpool preflight")?;
    let policy: Arc<dyn Policy> = Arc::new(build_execution_policy(file, manifest)?);
    let jupiter = build_jupiter_components(file)?;
    let vote_account = file
        .online
        .stake_vote_account
        .as_deref()
        .map(|value| parse_pubkey(value, "stake vote account"))
        .transpose()?;
    let mut engines = BTreeMap::new();
    for (agent_id, signer) in signers {
        let adapters =
            build_agent_adapters(file, gateway, &signer, jupiter.as_ref(), vote_account)?;
        let store_dyn: Arc<dyn StateStore> = store.clone();
        let gateway_concrete = Arc::clone(gateway);
        let gateway_dyn: Arc<dyn ChainGateway> = gateway_concrete;
        let engine = RuntimeEngine::new(
            store_dyn,
            gateway_dyn,
            Arc::clone(&policy),
            adapters,
            RuntimeSettings {
                adapter_context: adapter_context(file, gateway, signer.pubkey().to_string()),
                max_concurrency: 1,
            },
            Arc::new(NoFaults),
            Arc::clone(&clock),
        )
        .map_err(anyhow::Error::msg)?;
        engines.insert(agent_id, Arc::new(engine));
    }
    let store_dyn: Arc<dyn StateStore> = store;
    FleetRuntime::new(store_dyn, engines, file.core.runtime.max_concurrency, clock)
        .map(Arc::new)
        .map_err(anyhow::Error::msg)
}

fn build_agent_adapters(
    file: &FileConfig,
    gateway: &Arc<SolanaGateway>,
    signer: &Arc<LocalKeypair>,
    jupiter: Option<&(Arc<JupiterApiClient>, JupiterPolicy)>,
    vote_account: Option<Pubkey>,
) -> Result<Vec<(ActionKind, Arc<dyn ActionAdapter>)>> {
    let mut adapters: Vec<(ActionKind, Arc<dyn ActionAdapter>)> = Vec::new();
    for kind in &file.online.enabled_actions {
        let adapter: Arc<dyn ActionAdapter> = match kind {
            ActionKind::NativeTransfer => Arc::new(NativeTransferAdapter::new(
                Arc::clone(gateway),
                Arc::clone(signer),
            )),
            ActionKind::SplTransfer => Arc::new(SplTransferAdapter::new(
                Arc::clone(gateway),
                Arc::clone(signer),
            )),
            ActionKind::JupiterSwap => {
                let (client, policy) = jupiter.ok_or_else(|| {
                    anyhow::anyhow!("online Jupiter adapter configuration is missing")
                })?;
                Arc::new(JupiterAdapter::new(
                    Arc::clone(gateway),
                    Arc::clone(signer),
                    Arc::clone(client),
                    policy.clone(),
                ))
            }
            ActionKind::StakeLifecycle => Arc::new(NativeStakeAdapter::new(
                Arc::clone(gateway),
                Arc::clone(signer),
                vote_account.context("online stake vote account is missing")?,
            )),
            ActionKind::Idle => bail!("idle is not an executable adapter"),
        };
        adapters.push((*kind, adapter));
    }
    Ok(adapters)
}

fn build_execution_policy(file: &FileConfig, manifest: &FleetManifest) -> Result<SafetyPolicy> {
    let mut destinations: BTreeSet<_> = manifest
        .entries()
        .iter()
        .map(|entry| entry.public_key.clone())
        .collect();
    destinations.extend(
        file.online
            .spl_routes
            .iter()
            .map(|route| route.destination_owner.clone()),
    );
    let mut mints = BTreeSet::new();
    for route in &file.online.spl_routes {
        mints.insert(route.mint.clone());
    }
    for route in &file.online.swap_routes {
        mints.insert(route.input_mint.clone());
        mints.insert(route.output_mint.clone());
    }
    let native_input_mints = if mints.contains(WRAPPED_SOL_MINT) {
        BTreeSet::from([WRAPPED_SOL_MINT.to_owned()])
    } else {
        BTreeSet::new()
    };
    let maximum = file
        .offline
        .persona_configs()
        .into_iter()
        .map(|persona| persona.amount.max_amount)
        .max()
        .context("online runtime has no personas")?;
    let max_action_amounts = file
        .online
        .enabled_actions
        .iter()
        .map(|kind| (*kind, maximum))
        .collect();
    SafetyPolicy::new(PolicyConfig {
        budgets: file.core.budgets.clone(),
        allowed_actions: file.online.enabled_actions.clone(),
        max_action_amounts,
        allowed_destinations: destinations,
        allowed_mints: mints,
        native_input_mints,
        max_slippage_bps: file.offline.max_slippage_bps,
    })
    .map_err(anyhow::Error::msg)
}

fn build_jupiter_components(
    file: &FileConfig,
) -> Result<Option<(Arc<JupiterApiClient>, JupiterPolicy)>> {
    if !file
        .online
        .enabled_actions
        .contains(&ActionKind::JupiterSwap)
    {
        return Ok(None);
    }
    let config = file
        .online
        .jupiter
        .as_ref()
        .context("online Jupiter policy is missing")?;
    let mints = file
        .online
        .swap_routes
        .iter()
        .flat_map(|route| [&route.input_mint, &route.output_mint])
        .map(|value| parse_pubkey(value, "Jupiter mint"))
        .collect::<Result<BTreeSet<_>>>()?;
    let policy = JupiterPolicy::new(
        mints,
        config.route_label.clone(),
        pubkey_set(&config.allowed_amm_accounts, "Jupiter AMM")?,
        pubkey_set(&config.allowed_programs, "Jupiter program")?,
        pubkey_set(&config.allowed_accounts, "Jupiter account")?,
        pubkey_set(&config.allowed_lookup_tables, "Jupiter lookup table")?,
        config.max_price_impact_bps,
    )
    .map_err(anyhow::Error::msg)?
    .with_writable_accounts(pubkey_set(
        &config.allowed_writable_accounts,
        "Jupiter writable account",
    )?);
    let client = jupiter_client(config)?;
    Ok(Some((Arc::new(client), policy)))
}

fn jupiter_client(config: &JupiterFileConfig) -> Result<JupiterApiClient> {
    match config.api_key_env.as_deref() {
        Some(name) => {
            let key = env::var(name)
                .with_context(|| format!("Jupiter API key environment variable {name} is unset"))?;
            JupiterApiClient::production(key).map_err(anyhow::Error::msg)
        }
        None => JupiterApiClient::new(None).map_err(anyhow::Error::msg),
    }
}

#[allow(clippy::too_many_arguments)]
fn build_funding_runtime(
    file: &FileConfig,
    manifest: &FleetManifest,
    store: &Arc<Store>,
    gateway: &Arc<SolanaGateway>,
    funder: &Arc<LocalKeypair>,
    funding_agent_id: AgentId,
    amount: u64,
    clock: Arc<dyn Clock>,
) -> Result<Arc<FleetRuntime>> {
    let per_action = amount
        .checked_add(file.core.budgets.max_fee_lamports)
        .context("funding action ceiling overflow")?;
    let total = per_action
        .checked_mul(u64::try_from(manifest.entries().len())?)
        .context("funding budget overflow")?;
    let policy: Arc<dyn Policy> = Arc::new(
        SafetyPolicy::new(PolicyConfig {
            budgets: BudgetConfig {
                daily_lamports: total,
                lifetime_lamports: total,
                reserve_lamports: file.core.budgets.reserve_lamports,
                max_fee_lamports: file.core.budgets.max_fee_lamports,
                max_account_creation_lamports: 0,
            },
            allowed_actions: BTreeSet::from([ActionKind::NativeTransfer]),
            max_action_amounts: BTreeMap::from([(ActionKind::NativeTransfer, amount)]),
            allowed_destinations: manifest
                .entries()
                .iter()
                .map(|entry| entry.public_key.clone())
                .collect(),
            allowed_mints: BTreeSet::new(),
            native_input_mints: BTreeSet::new(),
            max_slippage_bps: 0,
        })
        .map_err(anyhow::Error::msg)?,
    );
    let adapter: Arc<dyn ActionAdapter> = Arc::new(NativeTransferAdapter::new(
        Arc::clone(gateway),
        Arc::clone(funder),
    ));
    let store_concrete = Arc::clone(store);
    let store_dyn: Arc<dyn StateStore> = store_concrete;
    let gateway_concrete = Arc::clone(gateway);
    let gateway_dyn: Arc<dyn ChainGateway> = gateway_concrete;
    let engine = RuntimeEngine::new(
        store_dyn.clone(),
        gateway_dyn,
        policy,
        [(ActionKind::NativeTransfer, adapter)],
        RuntimeSettings {
            adapter_context: adapter_context(file, gateway, funder.pubkey().to_string()),
            max_concurrency: 1,
        },
        Arc::new(NoFaults),
        Arc::clone(&clock),
    )
    .map_err(anyhow::Error::msg)?;
    FleetRuntime::new(
        store_dyn,
        BTreeMap::from([(funding_agent_id, Arc::new(engine))]),
        1,
        clock,
    )
    .map(Arc::new)
    .map_err(anyhow::Error::msg)
}

#[allow(clippy::too_many_arguments)]
fn register_funding_run(
    store: &Store,
    run_id: RunId,
    agent_id: AgentId,
    fleet_run_id: RunId,
    amount: u64,
    agents: usize,
    epoch: DateTime<Utc>,
) -> Result<bool> {
    let config = serde_json::json!({
        "purpose": "surfpool_fleet_funding",
        "fleet_run_id": fleet_run_id,
        "lamports_per_agent": amount,
        "agents": agents,
    });
    let config_bytes = serde_json::to_vec(&config).context("failed to hash funding config")?;
    let run_changed = store
        .register_run(&RunRegistration {
            id: run_id,
            model_version: FUNDING_MODEL_VERSION.to_owned(),
            config,
            config_hash: blake3::hash(&config_bytes).to_hex().to_string(),
            seed_hash: blake3::hash(b"surfpool-funding-has-no-random-seed")
                .to_hex()
                .to_string(),
            created_at: epoch,
        })
        .map_err(anyhow::Error::msg)?;
    let snapshot = AgentSnapshot {
        id: agent_id,
        run_id,
        next_sequence: 0,
        next_decision_at: epoch,
        budget_date: epoch.date_naive(),
        session_state: SessionState::Dormant,
        last_action_at: None,
        remaining_daily_budget: amount
            .checked_mul(u64::try_from(agents)?)
            .context("funding agent budget overflow")?,
        model_version: FUNDING_MODEL_VERSION.to_owned(),
    };
    let agent_changed = match store.get_agent(agent_id) {
        Ok(existing) if existing == snapshot => false,
        Ok(_) => bail!("funding agent metadata conflicts with its deterministic registration"),
        Err(CookerError::NotFound(_)) => {
            store
                .upsert_agent(&snapshot, epoch)
                .map_err(anyhow::Error::msg)?;
            true
        }
        Err(error) => return Err(error.into()),
    };
    Ok(run_changed || agent_changed)
}

fn funding_action(
    run_id: RunId,
    agent_id: AgentId,
    sequence: u64,
    destination: &str,
    lamports: u64,
    max_fee_lamports: u64,
    epoch: DateTime<Utc>,
) -> PlannedAction {
    PlannedAction {
        id: ActionId::derive(run_id, agent_id, sequence, FUNDING_MODEL_VERSION),
        run_id,
        agent_id,
        sequence,
        model_version: FUNDING_MODEL_VERSION.to_owned(),
        scheduled_at: epoch,
        payload: ActionPayload::NativeTransfer {
            destination: destination.to_owned(),
            lamports,
        },
        max_fee_lamports,
        max_account_creation_lamports: 0,
        created_at: epoch,
    }
}

async fn connect_surfpool(file: &FileConfig) -> Result<Arc<SolanaGateway>> {
    let endpoint = RpcEndpoint::new(file.core.network.rpc_url.clone())
        .map_err(anyhow::Error::msg)
        .context("RPC endpoint is not an explicit-loopback Surfpool URL")?;
    let gateway = Arc::new(
        SolanaGateway::connect(endpoint)
            .await
            .map_err(anyhow::Error::msg)
            .context("Surfpool identity preflight failed")?,
    );
    gateway
        .doctor()
        .await
        .map_err(anyhow::Error::msg)
        .context("repeated Surfpool identity probe failed")?;
    Ok(gateway)
}

fn adapter_context(file: &FileConfig, gateway: &SolanaGateway, signer: String) -> AdapterContext {
    AdapterContext {
        rpc_url: gateway.endpoint().as_url().clone(),
        signer,
        confirmation_timeout: Duration::from_secs(file.online.confirmation_timeout_seconds),
    }
}

fn load_public_manifest(file: &FileConfig, root: &Path) -> Result<FleetManifest> {
    let manifest = FleetManifest::load_public(root)
        .map_err(anyhow::Error::msg)
        .context("failed to load public fleet manifest")?;
    verify_manifest(file, &manifest)?;
    Ok(manifest)
}

fn load_private_manifest(file: &FileConfig, root: &Path) -> Result<FleetManifest> {
    let manifest = FleetManifest::load(root)
        .map_err(anyhow::Error::msg)
        .context("failed to validate fleet signer bindings after Surfpool preflight")?;
    verify_manifest(file, &manifest)?;
    Ok(manifest)
}

fn verify_manifest(file: &FileConfig, manifest: &FleetManifest) -> Result<()> {
    if manifest.surfnet_id() != file.core.network.surfnet_id {
        bail!("fleet manifest Surfnet identity differs from configuration");
    }
    if manifest.run_id() != crate::commands::execution_run_id(file) {
        bail!("fleet manifest run identity differs from configuration");
    }
    if file
        .online
        .enabled_actions
        .contains(&ActionKind::NativeTransfer)
        && manifest.entries().len() < 2
    {
        bail!("native fleet execution requires at least two distinct agent signers");
    }
    Ok(())
}

fn verify_execution_run(file: &FileConfig, manifest: &FleetManifest, store: &Store) -> Result<()> {
    let record = store
        .get_run(manifest.run_id())
        .map_err(anyhow::Error::msg)
        .context("fleet run metadata is missing")?;
    let config = serde_json::to_value(file).context("failed to encode run configuration")?;
    let bytes = serde_json::to_vec(&config).context("failed to hash run configuration")?;
    let expected_config_hash = blake3::hash(&bytes).to_hex().to_string();
    let expected_seed_hash = blake3::hash(file.core.seed_hex.as_bytes())
        .to_hex()
        .to_string();
    if record.model_version.as_deref() != Some(MODEL_VERSION)
        || record.config_hash.as_deref() != Some(expected_config_hash.as_str())
        || record.seed_hash.as_deref() != Some(expected_seed_hash.as_str())
    {
        bail!("runtime configuration differs from the immutable fleet run registration");
    }
    Ok(())
}

fn open_read_only_store(file: &FileConfig, database: &Path) -> Result<Store> {
    Store::open_read_only(database, store_identity(file)?)
        .map_err(anyhow::Error::msg)
        .context("failed to open runtime store read-only")
}

fn open_writable_store(file: &FileConfig, database: &Path) -> Result<Arc<Store>> {
    Store::open(database, store_identity(file)?)
        .map(Arc::new)
        .map_err(anyhow::Error::msg)
        .context("failed to open runtime store")
}

fn store_identity(file: &FileConfig) -> Result<StoreIdentity> {
    StoreIdentity::surfpool(file.core.network.surfnet_id.clone()).map_err(anyhow::Error::msg)
}

fn load_validated(path: &Path) -> Result<FileConfig> {
    let file = FileConfig::load(path)?;
    file.validate()
        .with_context(|| format!("configuration {} is invalid", path.display()))?;
    Ok(file)
}

fn canonical_root(path: &Path) -> Result<PathBuf> {
    path.canonicalize()
        .with_context(|| format!("invalid project root {}", path.display()))
}

fn resolve_path(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

fn require_database(database: &Path) -> Result<()> {
    if !database.is_file() {
        bail!("runtime database does not exist: {}", database.display());
    }
    Ok(())
}

fn kill_switch_path(file: &FileConfig, root: &Path) -> PathBuf {
    root.join(&file.online.kill_switch_file)
}

fn ensure_operator_stop_clear(file: &FileConfig, root: &Path) -> Result<()> {
    if file.core.runtime.kill_switch {
        bail!(
            "runtime kill switch is active; no network request, signer load, or state change occurred"
        );
    }
    let path = kill_switch_path(file, root);
    if path
        .try_exists()
        .with_context(|| format!("failed to inspect kill-switch path {}", path.display()))?
    {
        bail!(
            "filesystem kill switch {} is active; no network request, signer load, or state change occurred",
            path.display()
        );
    }
    Ok(())
}

fn spawn_operator_stop_monitor(path: PathBuf) -> (OperatorStopMonitor, watch::Receiver<bool>) {
    let (sender, receiver) = watch::channel(false);
    let reason = Arc::new(AtomicU8::new(STOP_NONE));
    let task_reason = Arc::clone(&reason);
    let task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(100));
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                signal = tokio::signal::ctrl_c() => {
                    task_reason.store(
                        if signal.is_ok() { STOP_CTRL_C } else { STOP_MONITOR_ERROR },
                        Ordering::SeqCst,
                    );
                    let _ = sender.send(true);
                    break;
                }
                _ = interval.tick() => {
                    match path.try_exists() {
                        Ok(false) => {}
                        Ok(true) => {
                            task_reason.store(STOP_FILE, Ordering::SeqCst);
                            let _ = sender.send(true);
                            break;
                        }
                        Err(_) => {
                            task_reason.store(STOP_MONITOR_ERROR, Ordering::SeqCst);
                            let _ = sender.send(true);
                            break;
                        }
                    }
                }
            }
        }
    });
    (OperatorStopMonitor { reason, task }, receiver)
}

fn fail_if_operator_stopped(monitor: &OperatorStopMonitor) -> Result<()> {
    if let Some(reason) = monitor.reason() {
        bail!("operator stop activated by {reason}; no new work was claimed");
    }
    Ok(())
}

const fn stop_reason(code: u8) -> Option<&'static str> {
    match code {
        STOP_CTRL_C => Some("ctrl_c"),
        STOP_FILE => Some("filesystem_kill_switch"),
        STOP_MONITOR_ERROR => Some("kill_switch_monitor_error"),
        _ => None,
    }
}

fn async_runtime(command: &str) -> Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .with_context(|| format!("failed to create {command} async runtime"))
}

fn lease_until(file: &FileConfig, now: DateTime<Utc>) -> Result<DateTime<Utc>> {
    let seconds = i64::try_from(file.core.runtime.lease_seconds)
        .context("runtime lease seconds do not fit i64")?;
    now.checked_add_signed(TimeDelta::seconds(seconds))
        .context("runtime lease expiry overflow")
}

fn audit_cutoff(now: DateTime<Utc>, seconds: u64) -> Result<DateTime<Utc>> {
    let seconds = i64::try_from(seconds).context("audit age does not fit i64")?;
    now.checked_sub_signed(TimeDelta::seconds(seconds))
        .context("audit cutoff underflow")
}

fn worker_id(prefix: &str) -> String {
    format!("cli-{prefix}-{}-{}", std::process::id(), Uuid::new_v4())
}

fn funding_run_id(fleet_run_id: RunId) -> RunId {
    const NAMESPACE: Uuid = Uuid::from_u128(0x575f_f512_7348_59b0_8411_57cc_a0fe_46d2);
    RunId(Uuid::new_v5(&NAMESPACE, fleet_run_id.0.as_bytes()))
}

fn parse_pubkey(value: &str, label: &str) -> Result<Pubkey> {
    Pubkey::from_str(value).with_context(|| format!("invalid {label} {value:?}"))
}

fn pubkey_set(values: &[String], label: &str) -> Result<BTreeSet<Pubkey>> {
    values
        .iter()
        .map(|value| parse_pubkey(value, label))
        .collect()
}

fn write_json(output: &mut impl Write, value: &impl Serialize) -> Result<()> {
    serde_json::to_writer_pretty(&mut *output, value).context("failed to write command JSON")?;
    writeln!(output).context("failed to terminate command JSON")
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;
    use tokio::time::timeout;

    use super::*;

    #[tokio::test]
    async fn filesystem_monitor_trips_the_worker_stop_channel() -> Result<()> {
        let directory = tempdir()?;
        let switch = directory.path().join("KILL_SWITCH");
        let (monitor, mut stop) = spawn_operator_stop_monitor(switch.clone());
        std::fs::write(&switch, b"stop\n")?;

        timeout(Duration::from_secs(2), stop.changed())
            .await
            .context("kill-switch monitor timed out")??;
        assert!(*stop.borrow());
        assert_eq!(monitor.reason(), Some("filesystem_kill_switch"));
        Ok(())
    }
}
