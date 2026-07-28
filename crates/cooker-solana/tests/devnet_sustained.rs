//! Sustained native-transfer run against public Solana devnet.
//!
//! `devnet_soak.rs` proves the engine's invariants on a public cluster inside one short bounded
//! window. This file answers a different question: what happens when the same engine keeps
//! running for hours instead of minutes. It drives the same store, runtime engine, safety
//! policy, and native-transfer adapter, but it paces work into fixed wall-clock rounds and
//! records a per-round measurement after every one of them.
//!
//! Three properties make the duration itself the result:
//!
//! - every round appends a complete line to a JSON Lines ledger and rewrites a self-consistent
//!   aggregate, so a run that is killed at any point still leaves committable evidence that
//!   describes exactly the rounds that finished;
//! - the round series carries the quantities a ten-minute run cannot show, namely database
//!   growth, resident-memory growth, per-round execution cost, and observed slot progression;
//! - each round measures the block leaders that actually held the slots the round spanned, so
//!   leader rotation coverage is measured rather than inferred from a slot count.
//!
//! Devnet is still not mainnet, and hours are still not days.

#![forbid(unsafe_code)]
#![recursion_limit = "256"]

use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::{Duration, Instant},
};

use chrono::{DateTime, TimeDelta, Utc};
use cooker_core::{
    ActionId, ActionPayload, ActionState, AgentId, PlannedAction, RunId, StateStore,
};
use cooker_runtime::{ExecutionResult, NoFaults};
use cooker_solana::{LocalKeypair, PublicCluster, RpcEndpoint, SolanaGateway};
use cooker_store::{RunRegistration, Store, StoreIdentity};
use serde_json::{Value, json};
use solana_pubkey::Pubkey;
use url::Url;

mod common;

use common::{
    NativeSoakParams, classify_unconfirmed_cause, env_u64, env_usize, native_transfer_runtime,
    write_json,
};

/// Wall-clock target for the whole run. Six hours is what the fixed, non-replenishable payer
/// balance and the endpoint's per-source rate budget allow while still leaving a large residue.
const DEFAULT_DURATION_SECS: u64 = 21_600;
/// Wall-clock spacing between round starts.
const DEFAULT_ROUND_INTERVAL_SECS: u64 = 60;
/// Actions submitted per steady-state round.
const DEFAULT_ROUND_BATCH: usize = 4;
/// Reused destinations. Creating a fresh destination per action would cost a rent-exempt
/// minimum every time, which the fixed payer balance cannot fund for hours.
const DEFAULT_POOL_SIZE: usize = 16;
/// Lamports each steady-state transfer moves into an already existing pool destination.
const TOPUP_LAMPORTS: u64 = 1_000;
const MAX_FEE_LAMPORTS: u64 = 10_000;
/// Lamports the safety policy refuses to spend below. The run's own floor stops it first.
const POLICY_RESERVE_LAMPORTS: u64 = 300_000_000;
/// Balance at which the run stops cleanly instead of continuing toward the policy reserve.
const RUN_BALANCE_FLOOR_LAMPORTS: u64 = 400_000_000;
const MODEL_VERSION: &str = "devnet-sustained-run-v1";
const SYSTEM_PROGRAM_ID: &str = "11111111111111111111111111111111";
const CONFIRMATION_TIMEOUT: Duration = Duration::from_secs(90);
/// Slots requested per leader window before the endpoint ceiling truncates the answer.
const MAX_LEADER_WINDOW_SLOTS: u64 = 5_000;
/// Reconciliation passes allowed in the final sweep before the run is declared unsettled.
const MAX_FINAL_RECONCILIATION_ROUNDS: usize = 8;
/// Pause between final sweep passes while a signature is neither visible nor past its deadline.
const FINAL_RECONCILIATION_BACKOFF: Duration = Duration::from_secs(45);
/// Rounds between in-process reconstructions of the store handle, signer, and runtime engine.
const RECONSTRUCT_EVERY_ROUNDS: usize = 60;
/// Fixed pool seed. Destinations are stable across runs so a reader can check any of them.
const POOL_SEED: &[u8; 24] = b"cooker-sustained-poolv1-";

/// One planned action plus the round that owns it.
struct ScheduledAction {
    action: PlannedAction,
    round: usize,
    destination_index: usize,
    lamports: u64,
}

/// Mutable per-action evidence, refreshed whenever the store touches that action.
struct ActionRecord {
    round: usize,
    destination_index: usize,
    planned_lamports: u64,
    state: &'static str,
    cause: Option<&'static str>,
    signature: Option<String>,
    slot: Option<u64>,
    fee_lamports: u64,
    events: usize,
    exact_source_debit: bool,
    exact_destination_credit: bool,
    budget_violation: bool,
}

impl ActionRecord {
    fn to_json(&self, sequence: u64) -> Value {
        json!({
            "sequence": sequence,
            "round": self.round,
            "state": self.state,
            "cause": self.cause,
            "signature": self.signature,
            "slot": self.slot,
            "fee_lamports": self.fee_lamports,
            "lamports": self.planned_lamports,
            "destination_index": self.destination_index,
        })
    }
}

/// Everything the aggregate needs that is accumulated as rounds finish.
#[derive(Default)]
struct Accumulator {
    records: BTreeMap<u64, ActionRecord>,
    signatures: BTreeSet<String>,
    confirmed_slots: BTreeSet<u64>,
    leaders: BTreeSet<String>,
    duplicate_signatures: usize,
    leader_changes: u64,
    leader_slots_measured: u64,
    leader_windows_measured: u64,
    leader_windows_unavailable: u64,
    last_leader: Option<String>,
    total_fees: u64,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "submits real transactions to public Solana devnet for hours and spends real devnet SOL"]
#[allow(
    clippy::too_many_lines,
    reason = "the sustained run keeps setup, the round loop, reconciliation, and evidence visible in one place"
)]
async fn sustained_devnet_run_holds_engine_invariants_across_hours()
-> Result<(), Box<dyn std::error::Error>> {
    let wall_started = Instant::now();
    let started_at = Utc::now();
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");

    let duration = Duration::from_secs(env_u64(
        "COOKER_SUSTAINED_DURATION_SECS",
        DEFAULT_DURATION_SECS,
    )?);
    let round_interval = Duration::from_secs(env_u64(
        "COOKER_SUSTAINED_ROUND_INTERVAL_SECS",
        DEFAULT_ROUND_INTERVAL_SECS,
    )?);
    let round_batch = env_usize("COOKER_SUSTAINED_ROUND_BATCH", DEFAULT_ROUND_BATCH)?;
    let pool_size = env_usize("COOKER_SUSTAINED_POOL_SIZE", DEFAULT_POOL_SIZE)?;
    if round_batch > 16 {
        return Err(io::Error::other("sustained round batch is limited to 16").into());
    }
    if pool_size > 64 {
        return Err(io::Error::other("sustained destination pool is limited to 64").into());
    }
    let steady_rounds = usize::try_from(duration.as_secs() / round_interval.as_secs().max(1))?;
    if steady_rounds == 0 {
        return Err(io::Error::other("sustained run needs at least one steady round").into());
    }

    let evidence_path = std::env::var_os("COOKER_SUSTAINED_EVIDENCE").map_or_else(
        || project_root.join("evidence/tmp/sustained-soak.json"),
        PathBuf::from,
    );
    let rounds_path = std::env::var_os("COOKER_SUSTAINED_ROUNDS").map_or_else(
        || project_root.join("evidence/tmp/sustained-rounds.jsonl"),
        PathBuf::from,
    );
    let database_path = std::env::var_os("COOKER_SUSTAINED_DATABASE").map_or_else(
        || project_root.join("evidence/tmp/sustained-soak.sqlite"),
        PathBuf::from,
    );
    if database_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "sustained run database already exists: {}",
                database_path.display()
            ),
        )
        .into());
    }
    for path in [&evidence_path, &rounds_path, &database_path] {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
    }
    let signer_path = std::env::var_os("COOKER_SUSTAINED_SIGNER_PATH").map_or_else(
        || project_root.join(".devnet/keys/payer.json"),
        PathBuf::from,
    );
    let rpc_url = std::env::var("COOKER_SUSTAINED_RPC_URL")
        .unwrap_or_else(|_| "https://api.devnet.solana.com".to_owned());

    // Leaving loopback requires naming the cluster in source. Configuration cannot do it.
    let endpoint = RpcEndpoint::public_cluster(Url::parse(&rpc_url)?, PublicCluster::Devnet)?;
    let gateway = Arc::new(SolanaGateway::connect_public_cluster(endpoint.clone()).await?);
    let cluster = gateway
        .cluster_identity()
        .ok_or_else(|| io::Error::other("sustained gateway did not prove a cluster identity"))?
        .clone();
    assert_eq!(cluster.cluster, PublicCluster::Devnet);
    assert_eq!(cluster.genesis_hash, PublicCluster::Devnet.genesis_hash());

    let mut signer = Arc::new(LocalKeypair::load(&project_root, &signer_path)?);
    let payer = signer.pubkey();
    let payer_address = payer.to_string();
    let payer_before = gateway.balance(&payer).await?;
    let epoch_before = gateway.epoch_info().await?;
    let create_lamports = gateway.minimum_balance_for_rent_exemption(0).await?;

    // Budget the whole plan up front against a balance that cannot be topped up.
    let steady_actions = steady_rounds
        .checked_mul(round_batch)
        .ok_or_else(|| io::Error::other("sustained action count overflowed"))?;
    let planned_count = steady_actions
        .checked_add(pool_size)
        .ok_or_else(|| io::Error::other("sustained action count overflowed"))?;
    let planned_lamports = u64::try_from(pool_size)?
        .checked_mul(create_lamports)
        .and_then(|pool| {
            u64::try_from(steady_actions)
                .ok()?
                .checked_mul(TOPUP_LAMPORTS)?
                .checked_add(pool)
        })
        .ok_or_else(|| io::Error::other("sustained transfer total overflowed"))?;
    let planned_fees = u64::try_from(planned_count)?
        .checked_mul(MAX_FEE_LAMPORTS)
        .ok_or_else(|| io::Error::other("sustained fee ceiling overflowed"))?;
    let planned_spend = planned_lamports
        .checked_add(planned_fees)
        .ok_or_else(|| io::Error::other("sustained spend ceiling overflowed"))?;
    let required_balance = planned_spend
        .checked_add(RUN_BALANCE_FLOOR_LAMPORTS)
        .ok_or_else(|| io::Error::other("sustained funding requirement overflowed"))?;
    if payer_before < required_balance {
        return Err(io::Error::other(format!(
            "sustained payer {payer_address} holds {payer_before} lamports but the plan needs \
             {required_balance} including the {RUN_BALANCE_FLOOR_LAMPORTS} lamport floor"
        ))
        .into());
    }

    let destinations: Vec<Pubkey> = (0..pool_size).map(pool_destination).collect();
    let destination_set: BTreeSet<String> = destinations
        .iter()
        .map(std::string::ToString::to_string)
        .collect();
    assert_eq!(destination_set.len(), pool_size);

    let run_id = RunId::new();
    // The plan's `scheduled_at` values and the round loop's wall-clock ticks must be
    // anchored to the same instant, or an early round claims nothing because its own
    // actions are not due yet.
    let plan_base = Utc::now();
    let plan_instant = Instant::now();
    let scheduled = plan_actions(
        run_id,
        plan_base,
        round_interval,
        steady_rounds,
        round_batch,
        &destinations,
        create_lamports,
    )?;
    assert_eq!(scheduled.len(), planned_count);
    let logical_ids: BTreeSet<ActionId> = scheduled
        .iter()
        .map(|entry| entry.action.id.clone())
        .collect();
    assert_eq!(logical_ids.len(), planned_count);
    let by_action: BTreeMap<ActionId, (u64, usize, usize, u64)> = scheduled
        .iter()
        .map(|entry| {
            (
                entry.action.id.clone(),
                (
                    entry.action.sequence,
                    entry.round,
                    entry.destination_index,
                    entry.lamports,
                ),
            )
        })
        .collect();

    let params = NativeSoakParams {
        model_version: MODEL_VERSION,
        transfer_lamports: create_lamports,
        max_fee_lamports: MAX_FEE_LAMPORTS,
        reserve_lamports: POLICY_RESERVE_LAMPORTS,
        confirmation_timeout: CONFIRMATION_TIMEOUT,
        max_concurrency: round_batch,
    };

    let mut store = Arc::new(Store::open(
        &database_path,
        StoreIdentity::new(PublicCluster::Devnet.as_str(), &cluster.genesis_hash)?,
    )?);
    assert_eq!(store.journal_mode()?.to_ascii_lowercase(), "wal");
    store.register_run(&RunRegistration {
        id: run_id,
        model_version: MODEL_VERSION.to_owned(),
        config: json!({
            "duration_secs": duration.as_secs(),
            "round_interval_secs": round_interval.as_secs(),
            "round_batch": round_batch,
            "pool_size": pool_size,
            "topup_lamports": TOPUP_LAMPORTS,
            "create_lamports": create_lamports,
            "max_fee_lamports": MAX_FEE_LAMPORTS,
            "rpc": endpoint.for_evidence(),
            "cluster": PublicCluster::Devnet.as_str(),
        }),
        config_hash: "d0d0c0de".to_owned(),
        seed_hash: "500a500a".to_owned(),
        created_at: plan_base,
    })?;
    let mut idempotent_enqueue_rejections = 0_usize;
    for entry in &scheduled {
        assert!(store.enqueue_action(&entry.action)?);
    }
    for entry in &scheduled {
        if !store.enqueue_action(&entry.action)? {
            idempotent_enqueue_rejections += 1;
        }
    }
    assert_eq!(idempotent_enqueue_rejections, planned_count);

    let mut runtime = native_transfer_runtime(
        Arc::clone(&store),
        &gateway,
        Arc::<SolanaGateway>::clone(&gateway),
        Arc::clone(&signer),
        &destination_set,
        params,
        Arc::new(NoFaults),
    )?;

    let mut accumulator = Accumulator::default();
    let mut rounds_completed = 0_usize;
    let mut reconstructions = 0_usize;
    let mut reconciled_confirmed = 0_usize;
    let mut reconciled_failed = 0_usize;
    let mut reconciled_expired = 0_usize;
    let mut in_round_reconciliation_passes = 0_usize;
    let mut peak_workers = 0_usize;
    // Time spent inside round execution, excluding the idle gap between rounds.
    let mut submission_ms = 0_u64;
    let mut worker_errors: Vec<String> = Vec::new();
    let mut round_series: Vec<Value> = Vec::new();
    let mut leader_cursor = epoch_before.absolute_slot;
    let mut stop_reason = "plan_completed";
    let mut payer_latest = payer_before;

    // The pool round funds every destination with one rent-exempt minimum, creating it when
    // it does not exist yet; the remaining rounds reuse those destinations.
    let total_rounds = steady_rounds
        .checked_add(1)
        .ok_or_else(|| io::Error::other("sustained round count overflowed"))?;
    for round in 0..total_rounds {
        let target = plan_instant + round_interval.saturating_mul(u32::try_from(round)?);
        let now = Instant::now();
        if target > now {
            tokio::time::sleep(target - now + Duration::from_secs(1)).await;
        }
        // The plan itself bounds the run. This guard only catches cumulative round overrun.
        if round > 0 && plan_instant.elapsed() >= duration + round_interval.saturating_mul(2) {
            stop_reason = "duration_guard_reached";
            break;
        }
        if payer_latest < RUN_BALANCE_FLOOR_LAMPORTS {
            stop_reason = "balance_floor_reached";
            break;
        }

        let round_started_at = Utc::now();
        let round_started = Instant::now();

        // Settle anything a previous round left ambiguous before adding new work. By now the
        // blockhash window of the earlier submission has advanced, so the answer is a result
        // rather than "come back later".
        let mut round_reconciled = 0_usize;
        let leases = store.claim_reconciliation_candidates(
            "sustained-reconciler",
            Utc::now(),
            Utc::now() + TimeDelta::minutes(30),
            round_batch * 4,
        )?;
        if !leases.is_empty() {
            in_round_reconciliation_passes += 1;
        }
        for lease in &leases {
            match runtime.reconcile(lease).await? {
                ExecutionResult::Confirmed { .. } | ExecutionResult::Audited { .. } => {
                    reconciled_confirmed += 1;
                    round_reconciled += 1;
                }
                ExecutionResult::Failed { .. } => {
                    reconciled_failed += 1;
                    round_reconciled += 1;
                }
                ExecutionResult::Expired { .. } => {
                    reconciled_expired += 1;
                    round_reconciled += 1;
                }
                _ => {}
            }
        }

        let claimed = store.claim_due_actions(
            &format!("sustained-round-{round}"),
            Utc::now(),
            Utc::now() + TimeDelta::minutes(30),
            round_batch.max(pool_size) * 2,
        )?;
        let claimed_ids: Vec<ActionId> = claimed
            .iter()
            .map(|lease| lease.action_id.clone())
            .collect();
        let claimed_count = claimed.len();
        let (_stop, stop_rx) = tokio::sync::watch::channel(false);
        let summary = Arc::clone(&runtime).run_claimed(claimed, stop_rx).await;
        let execution_ms = u64::try_from(round_started.elapsed().as_millis())?;
        submission_ms = submission_ms.saturating_add(execution_ms);
        peak_workers = peak_workers.max(summary.peak_workers);
        worker_errors.extend(summary.errors.iter().cloned());

        let touched: Vec<ActionId> = claimed_ids
            .into_iter()
            .chain(leases.iter().map(|lease| lease.action_id.clone()))
            .collect();
        for action_id in &touched {
            let Some(&(sequence, action_round, destination_index, lamports)) =
                by_action.get(action_id)
            else {
                continue;
            };
            let record = read_action_record(
                store.as_ref(),
                action_id,
                action_round,
                destination_index,
                lamports,
                &payer_address,
                &destinations,
                &mut accumulator,
            )?;
            accumulator.records.insert(sequence, record);
        }

        let epoch_after_round = gateway.epoch_info().await?;
        payer_latest = gateway.balance(&payer).await?;
        let status = store.status_snapshot(Utc::now(), None)?;
        let database = database_size(&database_path);
        let resident = resident_kib();
        let leader_window = epoch_after_round
            .absolute_slot
            .saturating_sub(leader_cursor);
        let leaders = if leader_window == 0 {
            Vec::new()
        } else {
            gateway
                .slot_leaders(leader_cursor, leader_window.min(MAX_LEADER_WINDOW_SLOTS))
                .await
                .unwrap_or_default()
        };
        let leader_window_available = !leaders.is_empty() || leader_window == 0;
        if leader_window > 0 {
            if leaders.is_empty() {
                accumulator.leader_windows_unavailable += 1;
            } else {
                accumulator.leader_windows_measured += 1;
                accumulator.leader_slots_measured += u64::try_from(leaders.len())?;
            }
        }
        let mut round_leader_changes = 0_u64;
        let mut round_leaders = BTreeSet::new();
        for leader in &leaders {
            let leader = leader.to_string();
            if accumulator.last_leader.as_ref() != Some(&leader) {
                if accumulator.last_leader.is_some() {
                    round_leader_changes += 1;
                }
                accumulator.last_leader = Some(leader.clone());
            }
            round_leaders.insert(leader.clone());
            accumulator.leaders.insert(leader);
        }
        accumulator.leader_changes += round_leader_changes;
        if !leaders.is_empty() {
            leader_cursor = leader_cursor.saturating_add(u64::try_from(leaders.len())?);
        }

        let tally = tally(&accumulator);
        rounds_completed += 1;
        let round_json = json!({
            "round": round,
            "phase": if round == 0 { "pool_funding" } else { "steady_state" },
            "started_at": round_started_at.to_rfc3339(),
            "finished_at": Utc::now().to_rfc3339(),
            "execution_ms": execution_ms,
            "claimed": claimed_count,
            "confirmed_in_round": summary.confirmed,
            "unknown_in_round": summary.unknown,
            "rejected_in_round": summary.rejected,
            "failed_in_round": summary.failed,
            "expired_in_round": summary.expired,
            "reconciled_in_round": round_reconciled,
            "ms_per_confirmed": per_confirmed(execution_ms, summary.confirmed),
            "cumulative_confirmed": tally.confirmed,
            "cumulative_unconfirmed": tally.unconfirmed(),
            "slot_before": leader_cursor.saturating_sub(u64::try_from(leaders.len())?),
            "slot_after": epoch_after_round.absolute_slot,
            "slots_advanced": leader_window,
            "epoch": epoch_after_round.epoch,
            "slot_index": epoch_after_round.slot_index,
            "leader_window_slots": leaders.len(),
            "leader_window_available": leader_window_available,
            "distinct_leaders_in_window": round_leaders.len(),
            "leader_changes_in_window": round_leader_changes,
            "distinct_leaders_so_far": accumulator.leaders.len(),
            "payer_lamports": payer_latest,
            "database_bytes": database.total,
            "database_main_bytes": database.main,
            "database_wal_bytes": database.wal,
            "journal_events": tally.events,
            "resident_kib": resident,
            "store_total_actions": status.total_actions,
            "store_due_actions": status.due_actions,
            "store_unresolved_actions": status.unresolved_actions,
            "store_active_leases": status.active_leases,
        });
        append_jsonl(&rounds_path, &round_json)?;
        round_series.push(round_json);

        write_json(
            &evidence_path,
            &aggregate(
                &AggregateInput {
                    completed: false,
                    stop_reason: "in_progress",
                    started_at,
                    wall_time_ms: u64::try_from(wall_started.elapsed().as_millis())?,
                    submission_ms,
                    duration_target_secs: duration.as_secs(),
                    round_interval_secs: round_interval.as_secs(),
                    round_batch,
                    pool_size,
                    create_lamports,
                    planned_count,
                    rounds_completed,
                    total_rounds,
                    reconstructions,
                    reconciled_confirmed,
                    reconciled_failed,
                    reconciled_expired,
                    in_round_reconciliation_passes,
                    final_reconciliation_rounds: 0,
                    peak_workers,
                    worker_errors: &worker_errors,
                    idempotent_enqueue_rejections,
                    payer_before,
                    payer_after: payer_latest,
                    epoch_before_epoch: epoch_before.epoch,
                    epoch_before_slot: epoch_before.absolute_slot,
                    epoch_after_epoch: epoch_after_round.epoch,
                    epoch_after_slot: epoch_after_round.absolute_slot,
                    database,
                    resident_kib: resident,
                    endpoint_for_evidence: endpoint.for_evidence(),
                    cluster_genesis: &cluster.genesis_hash,
                    solana_core: cluster.solana_core.as_deref(),
                    feature_set: cluster.feature_set,
                    payer_address: &payer_address,
                    destinations: &destinations,
                    rounds_file: rounds_path.file_name().map(|name| name.to_string_lossy()),
                },
                &accumulator,
                &round_series,
            ),
        )?;

        // Rebuild the store handle, signer, and runtime engine in place at a fixed cadence. The
        // OS process and the gateway object stay alive; durable state is reread from the same
        // WAL-mode database, which by now is hours old and much larger than at round zero.
        if round > 0 && round % RECONSTRUCT_EVERY_ROUNDS == 0 {
            drop(runtime);
            drop(store);
            drop(signer);
            signer = Arc::new(LocalKeypair::load(&project_root, &signer_path)?);
            store = Arc::new(Store::open(
                &database_path,
                StoreIdentity::new(PublicCluster::Devnet.as_str(), &cluster.genesis_hash)?,
            )?);
            store.verify_integrity()?;
            runtime = native_transfer_runtime(
                Arc::clone(&store),
                &gateway,
                Arc::<SolanaGateway>::clone(&gateway),
                Arc::clone(&signer),
                &destination_set,
                params,
                Arc::new(NoFaults),
            )?;
            reconstructions += 1;
        }
    }

    // Final sweep. Anything still ambiguous is settled by signature, never by resubmission.
    let mut final_reconciliation_rounds = 0_usize;
    loop {
        let leases = store.claim_reconciliation_candidates(
            "sustained-final-reconciler",
            Utc::now(),
            Utc::now() + TimeDelta::minutes(30),
            planned_count,
        )?;
        if leases.is_empty() {
            break;
        }
        final_reconciliation_rounds += 1;
        if final_reconciliation_rounds > MAX_FINAL_RECONCILIATION_ROUNDS {
            return Err(io::Error::other(format!(
                "sustained reconciliation did not settle within \
                 {MAX_FINAL_RECONCILIATION_ROUNDS} rounds"
            ))
            .into());
        }
        let mut still_ambiguous = 0_usize;
        for lease in &leases {
            match runtime.reconcile(lease).await? {
                ExecutionResult::Confirmed { .. } | ExecutionResult::Audited { .. } => {
                    reconciled_confirmed += 1;
                }
                ExecutionResult::Failed { .. } => reconciled_failed += 1,
                ExecutionResult::Expired { .. } => reconciled_expired += 1,
                ExecutionResult::Unknown { .. } | ExecutionResult::Orphaned { .. } => {
                    still_ambiguous += 1;
                }
                other => {
                    return Err(io::Error::other(format!(
                        "sustained reconciliation returned an unsettled result: {other:?}"
                    ))
                    .into());
                }
            }
        }
        for lease in &leases {
            let Some(&(sequence, action_round, destination_index, lamports)) =
                by_action.get(&lease.action_id)
            else {
                continue;
            };
            let record = read_action_record(
                store.as_ref(),
                &lease.action_id,
                action_round,
                destination_index,
                lamports,
                &payer_address,
                &destinations,
                &mut accumulator,
            )?;
            accumulator.records.insert(sequence, record);
        }
        if still_ambiguous > 0 {
            tokio::time::sleep(FINAL_RECONCILIATION_BACKOFF).await;
        }
    }

    let tally = tally(&accumulator);
    // Invariants that hold on any network, whatever the cluster does to individual transactions.
    assert!(worker_errors.is_empty(), "{worker_errors:?}");
    assert_eq!(accumulator.duplicate_signatures, 0);
    assert_eq!(tally.budget_violations, 0);
    assert_eq!(tally.unresolved_submitted, 0);
    assert_eq!(tally.unresolved_unknown, 0);
    assert_eq!(tally.exact_source_debits, tally.confirmed);
    assert_eq!(tally.exact_destination_credits, tally.confirmed);
    assert!(tally.events > 0);
    assert!(peak_workers <= round_batch);

    let payer_after = gateway.balance(&payer).await?;
    let epoch_after = gateway.epoch_info().await?;
    store.transition_run(run_id, "active", "completed", Utc::now())?;
    store.verify_integrity()?;

    let database = database_size(&database_path);
    let evidence = aggregate(
        &AggregateInput {
            completed: true,
            stop_reason,
            started_at,
            wall_time_ms: u64::try_from(wall_started.elapsed().as_millis())?,
            submission_ms,
            duration_target_secs: duration.as_secs(),
            round_interval_secs: round_interval.as_secs(),
            round_batch,
            pool_size,
            create_lamports,
            planned_count,
            rounds_completed,
            total_rounds,
            reconstructions,
            reconciled_confirmed,
            reconciled_failed,
            reconciled_expired,
            in_round_reconciliation_passes,
            final_reconciliation_rounds,
            peak_workers,
            worker_errors: &worker_errors,
            idempotent_enqueue_rejections,
            payer_before,
            payer_after,
            epoch_before_epoch: epoch_before.epoch,
            epoch_before_slot: epoch_before.absolute_slot,
            epoch_after_epoch: epoch_after.epoch,
            epoch_after_slot: epoch_after.absolute_slot,
            database,
            resident_kib: resident_kib(),
            endpoint_for_evidence: endpoint.for_evidence(),
            cluster_genesis: &cluster.genesis_hash,
            solana_core: cluster.solana_core.as_deref(),
            feature_set: cluster.feature_set,
            payer_address: &payer_address,
            destinations: &destinations,
            rounds_file: rounds_path.file_name().map(|name| name.to_string_lossy()),
        },
        &accumulator,
        &round_series,
    );
    write_json(&evidence_path, &evidence)?;
    println!("COOKER_SUSTAINED_EVIDENCE_PATH={}", evidence_path.display());
    println!("COOKER_SUSTAINED_ROUNDS_PATH={}", rounds_path.display());
    println!(
        "COOKER_SUSTAINED_SUMMARY rounds={rounds_completed} confirmed={} unconfirmed={} \
         stop_reason={stop_reason}",
        tally.confirmed,
        tally.unconfirmed()
    );
    Ok(())
}

/// Aggregated per-action state, recomputed from the record map whenever evidence is written.
#[derive(Default)]
struct Tally {
    confirmed: usize,
    failed: usize,
    expired: usize,
    rejected: usize,
    unresolved_submitted: usize,
    unresolved_unknown: usize,
    exact_source_debits: usize,
    exact_destination_credits: usize,
    budget_violations: usize,
    events: usize,
    causes: BTreeMap<&'static str, usize>,
}

impl Tally {
    const fn unconfirmed(&self) -> usize {
        self.failed + self.expired + self.rejected
    }

    const fn executed(&self) -> usize {
        self.confirmed + self.failed + self.expired + self.rejected
    }
}

fn tally(accumulator: &Accumulator) -> Tally {
    let mut tally = Tally::default();
    for record in accumulator.records.values() {
        tally.events += record.events;
        if record.exact_source_debit {
            tally.exact_source_debits += 1;
        }
        if record.exact_destination_credit {
            tally.exact_destination_credits += 1;
        }
        if record.budget_violation {
            tally.budget_violations += 1;
        }
        match record.state {
            "confirmed" => tally.confirmed += 1,
            "failed" => tally.failed += 1,
            "expired" => tally.expired += 1,
            "rejected" => tally.rejected += 1,
            "submitted" => tally.unresolved_submitted += 1,
            _ => tally.unresolved_unknown += 1,
        }
        if let Some(cause) = record.cause {
            *tally.causes.entry(cause).or_default() += 1;
        }
    }
    tally
}

/// Scalars the aggregate needs that are not derived from the accumulator.
struct AggregateInput<'a> {
    completed: bool,
    stop_reason: &'a str,
    started_at: DateTime<Utc>,
    wall_time_ms: u64,
    submission_ms: u64,
    duration_target_secs: u64,
    round_interval_secs: u64,
    round_batch: usize,
    pool_size: usize,
    create_lamports: u64,
    planned_count: usize,
    rounds_completed: usize,
    total_rounds: usize,
    reconstructions: usize,
    reconciled_confirmed: usize,
    reconciled_failed: usize,
    reconciled_expired: usize,
    in_round_reconciliation_passes: usize,
    final_reconciliation_rounds: usize,
    peak_workers: usize,
    worker_errors: &'a [String],
    idempotent_enqueue_rejections: usize,
    payer_before: u64,
    payer_after: u64,
    epoch_before_epoch: u64,
    epoch_before_slot: u64,
    epoch_after_epoch: u64,
    epoch_after_slot: u64,
    database: DatabaseSize,
    resident_kib: Option<u64>,
    endpoint_for_evidence: String,
    cluster_genesis: &'a str,
    solana_core: Option<&'a str>,
    feature_set: Option<u64>,
    payer_address: &'a str,
    destinations: &'a [Pubkey],
    rounds_file: Option<std::borrow::Cow<'a, str>>,
}

#[allow(
    clippy::too_many_lines,
    reason = "the aggregate is one flat evidence document and is clearer written in one place"
)]
fn aggregate(input: &AggregateInput<'_>, accumulator: &Accumulator, rounds: &[Value]) -> Value {
    let tally = tally(accumulator);
    let expected_debit = accumulator
        .records
        .values()
        .filter(|record| record.state == "confirmed")
        .try_fold(0_u64, |total, record| {
            total
                .checked_add(record.planned_lamports)?
                .checked_add(record.fee_lamports)
        });
    let payer_debit = input.payer_before.checked_sub(input.payer_after);
    let signatures: Vec<Value> = accumulator
        .records
        .iter()
        .map(|(sequence, record)| record.to_json(*sequence))
        .collect();
    json!({
        "schema_version": 1,
        "scenario": "sustained_devnet_native_transfer_run",
        "completed": input.completed,
        "stop_reason": input.stop_reason,
        "network": {
            "cluster": PublicCluster::Devnet.as_str(),
            "rpc": input.endpoint_for_evidence,
            "genesis_hash": input.cluster_genesis,
            "solana_core": input.solana_core,
            "feature_set": input.feature_set,
            "program_id": SYSTEM_PROGRAM_ID,
            "explorer_signature_url": "https://explorer.solana.com/tx/<signature>?cluster=devnet",
            "explorer_address_url": "https://explorer.solana.com/address/<address>?cluster=devnet",
        },
        "payer": input.payer_address,
        "destinations": input.destinations.iter().map(std::string::ToString::to_string)
            .collect::<Vec<_>>(),
        "round_series_file": input.rounds_file,
        "config": {
            "duration_target_secs": input.duration_target_secs,
            "round_interval_secs": input.round_interval_secs,
            "round_batch": input.round_batch,
            "pool_size": input.pool_size,
            "pool_create_lamports": input.create_lamports,
            "steady_topup_lamports": TOPUP_LAMPORTS,
            "max_fee_lamports": MAX_FEE_LAMPORTS,
            "policy_reserve_lamports": POLICY_RESERVE_LAMPORTS,
            "run_balance_floor_lamports": RUN_BALANCE_FLOOR_LAMPORTS,
            "reconstruct_every_rounds": RECONSTRUCT_EVERY_ROUNDS,
        },
        "started_at": input.started_at.to_rfc3339(),
        "finished_at": Utc::now().to_rfc3339(),
        "wall_time_ms": input.wall_time_ms,
        "wall_time_hours": round3(seconds(input.wall_time_ms) / 3600.0),
        "rounds_planned": input.total_rounds,
        "rounds_completed": input.rounds_completed,
        "planned_action_count": input.planned_count,
        "executed_action_count": tally.executed(),
        "logical_action_count": input.planned_count,
        "confirmed_action_count": tally.confirmed,
        "failed_action_count": tally.failed,
        "expired_action_count": tally.expired,
        "rejected_action_count": tally.rejected,
        "unconfirmed_action_count": tally.unconfirmed(),
        "unconfirmed_causes": tally.causes,
        "submission_wall_time_ms": input.submission_ms,
        "confirmed_per_wall_second": rate(tally.confirmed, input.wall_time_ms),
        "confirmed_per_submission_second": rate(tally.confirmed, input.submission_ms),
        "duplicate_logical_intents": 0,
        "duplicate_enqueue_attempts": input.planned_count,
        "idempotent_duplicate_enqueue_rejections": input.idempotent_enqueue_rejections,
        "duplicate_signatures": accumulator.duplicate_signatures,
        "budget_violations": tally.budget_violations,
        "unresolved_submitted": tally.unresolved_submitted,
        "unresolved_unknown": tally.unresolved_unknown,
        "worker_errors": input.worker_errors,
        "max_concurrency": input.round_batch,
        "peak_worker_count": input.peak_workers,
        "reconciliation": {
            "in_round_passes": input.in_round_reconciliation_passes,
            "final_passes": input.final_reconciliation_rounds,
            "reconciled_without_resend": input.reconciled_confirmed
                + input.reconciled_failed
                + input.reconciled_expired,
            "reconciled_to_confirmed": input.reconciled_confirmed,
            "reconciled_to_failed": input.reconciled_failed,
            "reconciled_to_expired": input.reconciled_expired,
        },
        "component_reconstruction": {
            "in_process_reconstructions": input.reconstructions,
            "os_process_restarts": 0,
            "runtime_engine_rebuilt": input.reconstructions > 0,
            "sqlite_database_reopened": input.reconstructions > 0,
            "signer_reloaded": input.reconstructions > 0,
            "gateway_reused": true,
        },
        "growth": growth(rounds, input.database, input.resident_kib, tally.confirmed),
        "degradation": degradation(rounds),
        "chain_observation": {
            "epoch_before": input.epoch_before_epoch,
            "epoch_after": input.epoch_after_epoch,
            "absolute_slot_before": input.epoch_before_slot,
            "absolute_slot_after": input.epoch_after_slot,
            "slots_spanned": input.epoch_after_slot.saturating_sub(input.epoch_before_slot),
            "distinct_confirmed_slots": accumulator.confirmed_slots.len(),
            "first_confirmed_slot": accumulator.confirmed_slots.iter().next(),
            "last_confirmed_slot": accumulator.confirmed_slots.iter().next_back(),
            "slot_leaders": {
                "windows_measured": accumulator.leader_windows_measured,
                "windows_unavailable": accumulator.leader_windows_unavailable,
                "slots_covered": accumulator.leader_slots_measured,
                "distinct_leaders": accumulator.leaders.len(),
                "leader_changes": accumulator.leader_changes,
            },
        },
        "state_delta": {
            "payer_before_lamports": input.payer_before,
            "payer_after_lamports": input.payer_after,
            "payer_debit_lamports": payer_debit,
            "fees_lamports": accumulator.total_fees,
            "expected_debit_lamports": expected_debit,
            "payer_equation_met": payer_debit.is_some() && payer_debit == expected_debit,
        },
        "exact_source_debit_count": tally.exact_source_debits,
        "exact_destination_credit_count": tally.exact_destination_credits,
        "journal_event_count": tally.events,
        "database_bytes": input.database.total,
        "database_main_bytes": input.database.main,
        "database_wal_bytes": input.database.wal,
        "resident_kib": input.resident_kib,
        "signature_order": "planned_sequence",
        "signatures": signatures,
    })
}

/// Database, journal, and resident-memory growth from the first completed round to the last.
fn growth(
    rounds: &[Value],
    database: DatabaseSize,
    resident_kib: Option<u64>,
    confirmed: usize,
) -> Value {
    let first_of = |field: &str| {
        rounds
            .first()
            .and_then(|round| round.get(field))
            .and_then(Value::as_u64)
    };
    let peak_of = |field: &'static str| {
        rounds
            .iter()
            .filter_map(|round| round.get(field).and_then(Value::as_u64))
            .max()
    };
    let resident_first = first_of("resident_kib");
    json!({
        "database_main_bytes_first_round": first_of("database_main_bytes"),
        "database_main_bytes_last_round": database.main,
        "database_wal_bytes_first_round": first_of("database_wal_bytes"),
        "database_wal_bytes_last_round": database.wal,
        "database_wal_bytes_peak": peak_of("database_wal_bytes"),
        "database_main_bytes_per_confirmed_action": if confirmed == 0 {
            None
        } else {
            Some(database.main / u64::try_from(confirmed).unwrap_or(u64::MAX))
        },
        "resident_kib_first_round": resident_first,
        "resident_kib_last_round": resident_kib,
        "resident_kib_peak": peak_of("resident_kib"),
        "resident_kib_delta": match (resident_first, resident_kib) {
            (Some(first), Some(last)) => Some(i128::from(last) - i128::from(first)),
            _ => None,
        },
    })
}

/// Compare the first tenth of the steady-state rounds against the last tenth.
///
/// This is a descriptive comparison of two windows of one run, not a fitted trend and not a
/// significance test.
fn degradation(rounds: &[Value]) -> Value {
    let steady: Vec<&Value> = rounds
        .iter()
        .filter(|round| round.get("phase").and_then(Value::as_str) == Some("steady_state"))
        .collect();
    if steady.len() < 20 {
        return json!({
            "comparable": false,
            "reason": "fewer than 20 steady-state rounds completed",
            "steady_rounds": steady.len(),
        });
    }
    let window = (steady.len() / 10).max(1);
    let head = window_median(&steady[..window], "execution_ms");
    let tail = window_median(&steady[steady.len() - window..], "execution_ms");
    json!({
        "comparable": true,
        "steady_rounds": steady.len(),
        "window_rounds": window,
        "execution_ms_median_first_window": head,
        "execution_ms_median_last_window": tail,
        "execution_ms_median_delta": match (head, tail) {
            (Some(head), Some(tail)) => Some(i128::from(tail) - i128::from(head)),
            _ => None,
        },
    })
}

fn window_median(rounds: &[&Value], field: &str) -> Option<u64> {
    let mut values: Vec<u64> = rounds
        .iter()
        .filter_map(|round| round.get(field).and_then(Value::as_u64))
        .collect();
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    values.get(values.len() / 2).copied()
}

/// Read one action's durable state and turn it into a committable record.
#[allow(
    clippy::too_many_arguments,
    reason = "the record is assembled from durable state plus the plan's own expectations"
)]
fn read_action_record(
    store: &Store,
    action_id: &ActionId,
    round: usize,
    destination_index: usize,
    planned_lamports: u64,
    payer_address: &str,
    destinations: &[Pubkey],
    accumulator: &mut Accumulator,
) -> Result<ActionRecord, Box<dyn std::error::Error>> {
    let state = store.action_state(action_id)?;
    let recovery = store.recovery_record(action_id, Utc::now())?;
    let cause = classify_unconfirmed_cause(&recovery.events);
    let signature = recovery
        .submission
        .as_ref()
        .map(|record| record.signature.clone());
    if let Some(signature) = signature.clone()
        && !accumulator.signatures.insert(signature)
    {
        accumulator.duplicate_signatures += 1;
    }
    let destination = destinations
        .get(destination_index)
        .ok_or_else(|| io::Error::other("destination index outside the pool"))?
        .to_string();

    let mut record = ActionRecord {
        round,
        destination_index,
        planned_lamports,
        state: "unknown",
        cause: None,
        signature,
        slot: None,
        fee_lamports: 0,
        events: recovery.events.len(),
        exact_source_debit: false,
        exact_destination_credit: false,
        budget_violation: false,
    };

    match state {
        ActionState::Confirmed => {
            record.state = "confirmed";
            let receipt = recovery
                .receipts
                .last()
                .ok_or_else(|| io::Error::other("confirmed action omitted its receipt journal"))?;
            assert!(receipt.postconditions_met);
            let fee = receipt
                .observations
                .iter()
                .find(|observation| observation.kind == "transaction_fee_ceiling")
                .and_then(|observation| observation.attributes.get("actual_fee_lamports"))
                .ok_or_else(|| io::Error::other("confirmed receipt omitted the exact fee"))?
                .parse::<u64>()?;
            record.fee_lamports = fee;
            if fee > MAX_FEE_LAMPORTS {
                record.budget_violation = true;
            }
            accumulator.total_fees = accumulator.total_fees.saturating_add(fee);
            record.exact_destination_credit = receipt.observations.iter().any(|observation| {
                observation.kind == "native_destination_balance_delta"
                    && observation.account == destination
                    && observation.expected_delta == Some(i128::from(planned_lamports))
                    && observation.attributes.get("actual_delta")
                        == Some(&planned_lamports.to_string())
                    && observation.attributes.get("met").map(String::as_str) == Some("true")
            });
            let exact_debit = planned_lamports
                .checked_add(fee)
                .ok_or_else(|| io::Error::other("per-transaction debit overflow"))?;
            record.exact_source_debit = receipt.observations.iter().any(|observation| {
                observation.kind == "native_source_balance_delta"
                    && observation.account == payer_address
                    && observation.attributes.get("actual_delta")
                        == Some(&format!("-{exact_debit}"))
                    && observation.attributes.get("met").map(String::as_str) == Some("true")
            });
            if let Some(slot) = receipt.slot {
                record.slot = Some(slot);
                accumulator.confirmed_slots.insert(slot);
            }
        }
        ActionState::Failed => {
            record.state = "failed";
            record.cause = Some(cause);
        }
        ActionState::Expired => {
            record.state = "expired";
            record.cause = Some(cause);
        }
        ActionState::Rejected => {
            record.state = "rejected";
            record.cause = Some(cause);
        }
        ActionState::Submitted => record.state = "submitted",
        _ => record.state = "unknown",
    }

    let usage = store.budget_usage(recovery.action.agent_id, Utc::now())?;
    let ceiling = planned_lamports.saturating_add(MAX_FEE_LAMPORTS);
    if usage.spent_today_lamports > ceiling || usage.spent_lifetime_lamports > ceiling {
        record.budget_violation = true;
    }
    Ok(record)
}

/// Build the whole run's plan: one pool-creation round, then fixed-size steady rounds.
fn plan_actions(
    run_id: RunId,
    base: DateTime<Utc>,
    round_interval: Duration,
    steady_rounds: usize,
    round_batch: usize,
    destinations: &[Pubkey],
    create_lamports: u64,
) -> Result<Vec<ScheduledAction>, Box<dyn std::error::Error>> {
    let interval = TimeDelta::from_std(round_interval)?;
    let mut planned = Vec::new();
    let mut sequence = 0_u64;
    for (index, destination) in destinations.iter().enumerate() {
        planned.push(scheduled_action(
            run_id,
            base,
            sequence,
            0,
            index,
            destination,
            create_lamports,
        ));
        sequence = sequence.saturating_add(1);
    }
    for round in 1..=steady_rounds {
        let scheduled_at = base + interval * i32::try_from(round)?;
        for slot in 0..round_batch {
            let index = usize::try_from(sequence)? % destinations.len();
            let destination = destinations
                .get(index)
                .ok_or_else(|| io::Error::other("destination index outside the pool"))?;
            let _ = slot;
            planned.push(scheduled_action(
                run_id,
                scheduled_at,
                sequence,
                round,
                index,
                destination,
                TOPUP_LAMPORTS,
            ));
            sequence = sequence.saturating_add(1);
        }
    }
    Ok(planned)
}

fn scheduled_action(
    run_id: RunId,
    scheduled_at: DateTime<Utc>,
    sequence: u64,
    round: usize,
    destination_index: usize,
    destination: &Pubkey,
    lamports: u64,
) -> ScheduledAction {
    let agent_id = AgentId::new();
    ScheduledAction {
        action: PlannedAction {
            id: ActionId::derive(run_id, agent_id, sequence, MODEL_VERSION),
            run_id,
            agent_id,
            sequence,
            model_version: MODEL_VERSION.to_owned(),
            scheduled_at,
            payload: ActionPayload::NativeTransfer {
                destination: destination.to_string(),
                lamports,
            },
            max_fee_lamports: MAX_FEE_LAMPORTS,
            max_account_creation_lamports: 0,
            created_at: scheduled_at,
        },
        round,
        destination_index,
        lamports,
    }
}

/// Derive a stable destination from a fixed seed so any reader can recompute the pool.
fn pool_destination(index: usize) -> Pubkey {
    let mut bytes = [0_u8; 32];
    bytes[..24].copy_from_slice(POOL_SEED);
    bytes[24..].copy_from_slice(&u64::try_from(index).unwrap_or(u64::MAX).to_le_bytes());
    Pubkey::new_from_array(bytes)
}

/// On-disk size of the store, split into the main database file and its WAL sidecar.
///
/// Reporting only one of them would mislead in opposite directions. In WAL mode the main file
/// grows in steps at checkpoints, while the log itself sawtooths: it fills, is checkpointed back
/// into the main file, and starts over. Durable growth is the main file; the log is working set.
#[derive(Clone, Copy, Debug, Default)]
struct DatabaseSize {
    main: u64,
    wal: u64,
    total: u64,
}

fn database_size(path: &Path) -> DatabaseSize {
    let file_len = |path: PathBuf| {
        fs::metadata(path)
            .map(|meta| meta.len())
            .unwrap_or_default()
    };
    let sidecar = |suffix: &str| {
        let mut name = path.as_os_str().to_os_string();
        name.push(suffix);
        file_len(PathBuf::from(name))
    };
    let main = file_len(path.to_path_buf());
    let wal = sidecar("-wal");
    let shm = sidecar("-shm");
    DatabaseSize {
        main,
        wal,
        total: main.saturating_add(wal).saturating_add(shm),
    }
}

/// Current resident set size in KiB as the operating system reports it.
fn resident_kib() -> Option<u64> {
    let pid = std::process::id().to_string();
    let output = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid])
        .output()
        .ok()?;
    String::from_utf8(output.stdout).ok()?.trim().parse().ok()
}

/// Append one complete JSON object as a line, so a killed run still leaves whole records.
fn append_jsonl(path: &Path, value: &Value) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::Write as _;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let mut line = serde_json::to_vec(value)?;
    line.push(b'\n');
    file.write_all(&line)?;
    file.flush()?;
    file.sync_data()?;
    Ok(())
}

#[allow(
    clippy::cast_precision_loss,
    reason = "reported rates are observations, not exact quantities"
)]
fn seconds(millis: u64) -> f64 {
    millis as f64 / 1000.0
}

#[allow(
    clippy::cast_precision_loss,
    reason = "reported rates are observations, not exact quantities"
)]
fn rate(count: usize, millis: u64) -> f64 {
    if millis == 0 {
        return 0.0;
    }
    round3(count as f64 * 1000.0 / millis as f64)
}

#[allow(
    clippy::cast_precision_loss,
    reason = "reported rates are observations, not exact quantities"
)]
fn per_confirmed(millis: u64, confirmed: usize) -> Option<f64> {
    if confirmed == 0 {
        return None;
    }
    Some(round3(millis as f64 / confirmed as f64))
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}
