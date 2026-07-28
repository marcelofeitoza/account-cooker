//! Bounded native-transfer run against public Solana devnet.
//!
//! This is the public-network counterpart to `surfpool_soak.rs`. It drives the same store,
//! runtime engine, safety policy, and native-transfer adapter, but every transaction is signed
//! locally and submitted to a real cluster whose genesis hash is proven before a signer is
//! loaded. The invariants that can hold on any network are asserted; the quantities that depend
//! on network topology are measured and written to evidence rather than asserted away.
//!
//! The run is deliberately bounded. It is not a sustained-load proof and devnet is not mainnet.

#![forbid(unsafe_code)]
#![recursion_limit = "256"]

use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::PathBuf,
    str::FromStr,
    sync::Arc,
    time::{Duration, Instant},
};

use chrono::{TimeDelta, Utc};
use cooker_core::{
    ActionId, ActionPayload, ActionState, ChainGateway, ChainReceipt, CookerError, RunId,
    SimulationReceipt, StateStore,
};
use cooker_runtime::{ExecutionResult, NoFaults};
use cooker_solana::{LocalKeypair, PublicCluster, RpcEndpoint, SolanaGateway};
use cooker_store::{RunRegistration, Store, StoreIdentity};
use serde_json::json;
use solana_pubkey::Pubkey;
use solana_signature::Signature;
use tokio::sync::watch;
use url::Url;

mod common;

use common::{
    LoseOneSendResponse, NativeSoakParams, env_usize, native_transfer_runtime,
    planned_native_actions, write_json,
};

const DEFAULT_TRANSACTION_COUNT: usize = 200;
const DEFAULT_CONCURRENCY: usize = 8;
const DEFAULT_RPC_URL: &str = "https://api.devnet.solana.com";
const TRANSFER_LAMPORTS: u64 = 1_000_000;
const MAX_FEE_LAMPORTS: u64 = 10_000;
const RESERVE_LAMPORTS: u64 = 100_000_000;
const MODEL_VERSION: &str = "devnet-bounded-run-v2";
const SYSTEM_PROGRAM_ID: &str = "11111111111111111111111111111111";
const CONFIRMATION_TIMEOUT: Duration = Duration::from_secs(90);
/// Reconciliation passes allowed before the run is declared unsettled.
const MAX_RECONCILIATION_ROUNDS: usize = 6;
/// Pause between passes when a signature is still neither visible nor past its deadline.
///
/// A 45-second pause normally advances the validity window substantially. The bounded loop
/// permits multiple passes because public slot progression is external.
const RECONCILIATION_BACKOFF: Duration = Duration::from_secs(45);
const RUN: NativeSoakParams = NativeSoakParams {
    model_version: MODEL_VERSION,
    transfer_lamports: TRANSFER_LAMPORTS,
    max_fee_lamports: MAX_FEE_LAMPORTS,
    reserve_lamports: RESERVE_LAMPORTS,
    confirmation_timeout: CONFIRMATION_TIMEOUT,
    max_concurrency: DEFAULT_CONCURRENCY,
};

/// Gateway wrapper that waits for a submitted signature to become visible before returning.
///
/// Without this barrier the injected response loss could fire before the cluster has any record
/// of the transaction, which would test a different failure than the one being claimed.
#[derive(Debug)]
struct ConfirmSubmissionGateway {
    inner: Arc<SolanaGateway>,
}

#[async_trait::async_trait]
impl ChainGateway for ConfirmSubmissionGateway {
    async fn simulate(&self, transaction: &[u8]) -> Result<SimulationReceipt, CookerError> {
        ChainGateway::simulate(self.inner.as_ref(), transaction).await
    }

    async fn submit(&self, transaction: &[u8]) -> Result<String, CookerError> {
        let signature = ChainGateway::submit(self.inner.as_ref(), transaction).await?;
        let parsed = Signature::from_str(&signature)
            .map_err(|error| CookerError::Codec(format!("invalid signature: {error}")))?;
        let started = Instant::now();
        loop {
            if let Some(status) = self.inner.signature_status(&parsed).await? {
                if let Some(error) = status.error {
                    return Err(CookerError::Chain(format!(
                        "devnet submission failed before response loss: {error}"
                    )));
                }
                break;
            }
            if started.elapsed() >= CONFIRMATION_TIMEOUT {
                return Err(CookerError::UnknownOutcome(
                    "devnet submission did not become visible before the barrier timeout"
                        .to_owned(),
                ));
            }
            tokio::time::sleep(Duration::from_millis(1_500)).await;
        }
        Ok(signature)
    }

    async fn observe(
        &self,
        action_id: &ActionId,
        signature: &str,
    ) -> Result<ChainReceipt, CookerError> {
        ChainGateway::observe(self.inner.as_ref(), action_id, signature).await
    }

    async fn native_balance(&self, address: &str) -> Result<u64, CookerError> {
        ChainGateway::native_balance(self.inner.as_ref(), address).await
    }

    async fn block_height(&self) -> Result<u64, CookerError> {
        ChainGateway::block_height(self.inner.as_ref()).await
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "submits real transactions to public Solana devnet and spends real devnet SOL"]
#[allow(
    clippy::too_many_lines,
    reason = "the devnet run keeps setup, execution, reconciliation, and evidence visible in one place"
)]
async fn bounded_devnet_run_confirms_and_reconciles_without_duplicate_intents()
-> Result<(), Box<dyn std::error::Error>> {
    let wall_started = Instant::now();
    let started_at = Utc::now();
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let transaction_count = env_usize("COOKER_DEVNET_TRANSACTIONS", DEFAULT_TRANSACTION_COUNT)?;
    let max_concurrency = env_usize("COOKER_DEVNET_CONCURRENCY", DEFAULT_CONCURRENCY)?;
    if max_concurrency > 32 {
        return Err(io::Error::other("devnet run concurrency is limited to 32").into());
    }
    let evidence_path = std::env::var_os("COOKER_DEVNET_EVIDENCE").map_or_else(
        || project_root.join("evidence/tmp/devnet-soak.json"),
        PathBuf::from,
    );
    let database_path = std::env::var_os("COOKER_DEVNET_DATABASE").map_or_else(
        || project_root.join("evidence/tmp/devnet-soak.sqlite"),
        PathBuf::from,
    );
    if database_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "devnet run database already exists: {}",
                database_path.display()
            ),
        )
        .into());
    }
    if let Some(parent) = database_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let signer_path = std::env::var_os("COOKER_DEVNET_SIGNER_PATH").map_or_else(
        || project_root.join(".devnet/keys/payer.json"),
        PathBuf::from,
    );
    let rpc_url =
        std::env::var("COOKER_DEVNET_RPC_URL").unwrap_or_else(|_| DEFAULT_RPC_URL.to_owned());

    // Leaving loopback requires naming the cluster in source. Configuration cannot do it.
    let endpoint = RpcEndpoint::public_cluster(Url::parse(&rpc_url)?, PublicCluster::Devnet)?;
    let gateway = Arc::new(SolanaGateway::connect_public_cluster(endpoint.clone()).await?);
    let cluster = gateway
        .cluster_identity()
        .ok_or_else(|| io::Error::other("devnet gateway did not prove a cluster identity"))?
        .clone();
    assert_eq!(cluster.cluster, PublicCluster::Devnet);
    assert_eq!(cluster.genesis_hash, PublicCluster::Devnet.genesis_hash());

    let signer = Arc::new(LocalKeypair::load(&project_root, &signer_path)?);
    let payer = signer.pubkey();
    let payer_address = payer.to_string();
    let payer_before = gateway.balance(&payer).await?;
    let required_balance = u64::try_from(transaction_count)?
        .checked_mul(TRANSFER_LAMPORTS + MAX_FEE_LAMPORTS)
        .and_then(|total| total.checked_add(RESERVE_LAMPORTS))
        .ok_or_else(|| io::Error::other("devnet funding requirement overflowed"))?;
    if payer_before < required_balance {
        return Err(io::Error::other(format!(
            "devnet payer {payer_address} holds {payer_before} lamports but the run needs {required_balance}"
        ))
        .into());
    }
    let epoch_before = gateway.epoch_info().await?;

    let run_id = RunId::new();
    let scheduled_at = Utc::now();
    let actions =
        planned_native_actions(run_id, scheduled_at, transaction_count, RUN, |sequence| {
            run_scoped_destination(run_id, sequence)
        });
    let destinations: BTreeSet<String> = actions
        .iter()
        .filter_map(|action| match &action.payload {
            ActionPayload::NativeTransfer { destination, .. } => Some(destination.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(destinations.len(), transaction_count);
    let logical_ids: BTreeSet<_> = actions.iter().map(|action| action.id.clone()).collect();
    assert_eq!(logical_ids.len(), transaction_count);

    let store = Arc::new(Store::open(
        &database_path,
        StoreIdentity::new(PublicCluster::Devnet.as_str(), &cluster.genesis_hash)?,
    )?);
    assert_eq!(store.journal_mode()?.to_ascii_lowercase(), "wal");
    store.register_run(&RunRegistration {
        id: run_id,
        model_version: MODEL_VERSION.to_owned(),
        config: json!({
            "transactions": transaction_count,
            "transfer_lamports": TRANSFER_LAMPORTS,
            "max_fee_lamports": MAX_FEE_LAMPORTS,
            "max_concurrency": max_concurrency,
            "rpc": endpoint.for_evidence(),
            "cluster": PublicCluster::Devnet.as_str(),
        }),
        config_hash: "d0d0c0de".to_owned(),
        seed_hash: "500a500a".to_owned(),
        created_at: scheduled_at,
    })?;
    for action in &actions {
        assert!(store.enqueue_action(action)?);
    }
    let mut idempotent_enqueue_rejections = 0_usize;
    for action in &actions {
        if !store.enqueue_action(action)? {
            idempotent_enqueue_rejections += 1;
        }
    }
    assert_eq!(idempotent_enqueue_rejections, transaction_count);

    // A small barriered prefix carries the injected response loss. Most actions run without
    // that visibility barrier, but the reported end-to-end interval includes both batches.
    let first_batch_size = (transaction_count / 10).max(4).min(transaction_count);
    let loss = Arc::new(LoseOneSendResponse::new(
        "injected devnet send-response loss",
    ));
    let first_runtime = native_transfer_runtime(
        Arc::clone(&store),
        &gateway,
        Arc::new(ConfirmSubmissionGateway {
            inner: Arc::clone(&gateway),
        }),
        Arc::clone(&signer),
        &destinations,
        NativeSoakParams {
            max_concurrency,
            ..RUN
        },
        loss.clone(),
    )?;
    let first_claimed = store.claim_due_actions(
        "devnet-before-reconstruction",
        Utc::now(),
        Utc::now() + TimeDelta::minutes(60),
        first_batch_size,
    )?;
    assert_eq!(first_claimed.len(), first_batch_size);
    let (_first_stop, first_stop_rx) = watch::channel(false);
    let first_started = Instant::now();
    let first_summary = Arc::clone(&first_runtime)
        .run_claimed(first_claimed, first_stop_rx)
        .await;
    let first_elapsed = first_started.elapsed();
    assert!(first_summary.errors.is_empty(), "{first_summary:?}");
    // At least the injected loss must land in the ambiguous bucket. A public endpoint may add
    // more, and every one of them is counted and reconciled rather than assumed away.
    assert!(first_summary.unknown >= 1, "{first_summary:?}");
    assert!(loss.triggered());

    // Rebuild the runtime engine, store handle, and signer inside this process. The original
    // SolanaGateway object and OS process remain alive. Durable action state is read from the
    // same SQLite database, which remains configured in WAL mode.
    drop(first_runtime);
    drop(store);
    drop(signer);

    let signer = Arc::new(LocalKeypair::load(&project_root, &signer_path)?);
    let store = Arc::new(Store::open(
        &database_path,
        StoreIdentity::new(PublicCluster::Devnet.as_str(), &cluster.genesis_hash)?,
    )?);
    store.verify_integrity()?;
    let second_runtime = native_transfer_runtime(
        Arc::clone(&store),
        &gateway,
        Arc::<SolanaGateway>::clone(&gateway),
        Arc::clone(&signer),
        &destinations,
        NativeSoakParams {
            max_concurrency,
            ..RUN
        },
        Arc::new(NoFaults),
    )?;
    let second_claimed = store.claim_due_actions(
        "devnet-after-reconstruction",
        Utc::now(),
        Utc::now() + TimeDelta::minutes(60),
        transaction_count,
    )?;
    assert_eq!(second_claimed.len(), transaction_count - first_batch_size);
    let (_second_stop, second_stop_rx) = watch::channel(false);
    let second_started = Instant::now();
    let second_summary = Arc::clone(&second_runtime)
        .run_claimed(second_claimed, second_stop_rx)
        .await;
    let second_elapsed = second_started.elapsed();
    assert!(second_summary.errors.is_empty(), "{second_summary:?}");

    // Reconcile every ambiguous outcome by signature. Nothing is ever resubmitted.
    //
    // A public cluster can legitimately leave a signature unresolved for a while: it is neither
    // visible yet nor past its blockhash deadline. That is a "come back later" answer, not a
    // result, so the loop waits out the remaining validity window instead of forcing a verdict.
    let mut reconciled_confirmed = 0_usize;
    let mut reconciled_failed = 0_usize;
    let mut reconciled_expired = 0_usize;
    let mut reconciliation_rounds = 0_usize;
    loop {
        let leases = store.claim_reconciliation_candidates(
            "devnet-reconciler",
            Utc::now(),
            Utc::now() + TimeDelta::minutes(60),
            transaction_count,
        )?;
        if leases.is_empty() {
            break;
        }
        reconciliation_rounds += 1;
        if reconciliation_rounds > MAX_RECONCILIATION_ROUNDS {
            return Err(io::Error::other(format!(
                "devnet reconciliation did not settle within {MAX_RECONCILIATION_ROUNDS} rounds"
            ))
            .into());
        }
        let mut still_ambiguous = 0_usize;
        for lease in leases {
            match second_runtime.reconcile(&lease).await? {
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
                        "devnet reconciliation returned an unsettled result: {other:?}"
                    ))
                    .into());
                }
            }
        }
        if still_ambiguous > 0 {
            tokio::time::sleep(RECONCILIATION_BACKOFF).await;
        }
    }
    assert!(
        reconciled_confirmed >= 1,
        "the injected response loss must reconcile to a confirmation without resubmission"
    );

    let remaining_due = store.claim_due_actions(
        "devnet-final-due-check",
        Utc::now(),
        Utc::now() + TimeDelta::minutes(5),
        transaction_count,
    )?;
    assert!(remaining_due.is_empty());

    let mut confirmed = 0_usize;
    let mut failed = 0_usize;
    let mut expired = 0_usize;
    let mut rejected = 0_usize;
    let mut unresolved_submitted = 0_usize;
    let mut unresolved_unknown = 0_usize;
    let mut duplicate_signatures = 0_usize;
    let mut budget_violations = 0_usize;
    let mut exact_source_debits = 0_usize;
    let mut exact_destination_credits = 0_usize;
    let mut journal_events = 0_usize;
    let mut total_fees = 0_u64;
    let mut signatures = BTreeSet::new();
    let mut slots = BTreeSet::new();
    let mut unconfirmed_causes: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut transactions = Vec::with_capacity(transaction_count);
    for action in &actions {
        let state = store.action_state(&action.id)?;
        let recovery = store.recovery_record(&action.id, Utc::now())?;
        journal_events += recovery.events.len();
        let cause = classify_unconfirmed_cause(&recovery.events);
        let signature = recovery.submission.as_ref().map(|record| {
            if !signatures.insert(record.signature.clone()) {
                duplicate_signatures += 1;
            }
            record.signature.clone()
        });
        let ActionPayload::NativeTransfer { destination, .. } = &action.payload else {
            return Err(io::Error::other("devnet action was not a native transfer").into());
        };
        match state {
            ActionState::Confirmed => {
                confirmed += 1;
                let receipt = recovery.receipts.last().ok_or_else(|| {
                    io::Error::other("confirmed action omitted its receipt journal")
                })?;
                assert!(receipt.postconditions_met);
                let fee = receipt
                    .observations
                    .iter()
                    .find(|observation| observation.kind == "transaction_fee_ceiling")
                    .and_then(|observation| observation.attributes.get("actual_fee_lamports"))
                    .ok_or_else(|| io::Error::other("confirmed receipt omitted the exact fee"))?
                    .parse::<u64>()?;
                if fee > MAX_FEE_LAMPORTS {
                    budget_violations += 1;
                }
                total_fees = total_fees
                    .checked_add(fee)
                    .ok_or_else(|| io::Error::other("total fee overflow"))?;
                let destination_observation = receipt
                    .observations
                    .iter()
                    .find(|observation| {
                        observation.kind == "native_destination_balance_delta"
                            && observation.account == *destination
                    })
                    .ok_or_else(|| io::Error::other("receipt omitted destination state delta"))?;
                if destination_observation.expected_delta == Some(i128::from(TRANSFER_LAMPORTS))
                    && destination_observation.attributes.get("actual_delta")
                        == Some(&TRANSFER_LAMPORTS.to_string())
                    && destination_observation
                        .attributes
                        .get("met")
                        .map(String::as_str)
                        == Some("true")
                {
                    exact_destination_credits += 1;
                }
                let exact_debit = TRANSFER_LAMPORTS
                    .checked_add(fee)
                    .ok_or_else(|| io::Error::other("per-transaction debit overflow"))?;
                let source_observation = receipt
                    .observations
                    .iter()
                    .find(|observation| {
                        observation.kind == "native_source_balance_delta"
                            && observation.account == payer_address
                    })
                    .ok_or_else(|| io::Error::other("receipt omitted source state delta"))?;
                if source_observation.attributes.get("actual_delta")
                    == Some(&format!("-{exact_debit}"))
                    && source_observation.attributes.get("met").map(String::as_str) == Some("true")
                {
                    exact_source_debits += 1;
                }
                if let Some(slot) = receipt.slot {
                    slots.insert(slot);
                }
                transactions.push(json!({
                    "sequence": action.sequence,
                    "state": "confirmed",
                    "signature": signature,
                    "slot": receipt.slot,
                    "fee_lamports": fee,
                    "destination": destination,
                }));
            }
            ActionState::Failed => {
                failed += 1;
                *unconfirmed_causes.entry(cause).or_default() += 1;
                transactions.push(json!({
                    "sequence": action.sequence,
                    "state": "failed",
                    "cause": cause,
                    "signature": signature,
                    "slot": serde_json::Value::Null,
                    "fee_lamports": 0,
                    "destination": destination,
                }));
            }
            // Signed and persisted, with no signature observed through blockhash expiry. The
            // record proves absence, not whether the endpoint or cluster received the send.
            ActionState::Expired => {
                expired += 1;
                *unconfirmed_causes.entry(cause).or_default() += 1;
                transactions.push(json!({
                    "sequence": action.sequence,
                    "state": "expired",
                    "cause": cause,
                    "signature": signature,
                    "slot": serde_json::Value::Null,
                    "fee_lamports": 0,
                    "destination": destination,
                }));
            }
            ActionState::Rejected => {
                rejected += 1;
                *unconfirmed_causes.entry(cause).or_default() += 1;
                transactions.push(json!({
                    "sequence": action.sequence,
                    "state": "rejected",
                    "cause": cause,
                    "signature": signature,
                    "slot": serde_json::Value::Null,
                    "fee_lamports": 0,
                    "destination": destination,
                }));
            }
            ActionState::Submitted => unresolved_submitted += 1,
            ActionState::Unknown => unresolved_unknown += 1,
            other => {
                return Err(io::Error::other(format!(
                    "devnet action {} ended in {other:?}",
                    action.id
                ))
                .into());
            }
        }
        let usage = store.budget_usage(action.agent_id, Utc::now())?;
        let action_ceiling = TRANSFER_LAMPORTS + MAX_FEE_LAMPORTS;
        if usage.spent_today_lamports > action_ceiling
            || usage.spent_lifetime_lamports > action_ceiling
        {
            budget_violations += 1;
        }
    }

    // Invariants that hold on any network, whatever the cluster does to individual transactions.
    assert_eq!(confirmed + failed + expired + rejected, transaction_count);
    assert_eq!(duplicate_signatures, 0);
    assert_eq!(budget_violations, 0);
    assert_eq!(unresolved_submitted, 0);
    assert_eq!(unresolved_unknown, 0);
    assert_eq!(exact_source_debits, confirmed);
    assert_eq!(exact_destination_credits, confirmed);
    assert!(journal_events > 0);

    let payer_after = gateway.balance(&payer).await?;
    let epoch_after = gateway.epoch_info().await?;
    let payer_debit = payer_before
        .checked_sub(payer_after)
        .ok_or_else(|| io::Error::other("devnet payer balance unexpectedly increased"))?;
    let confirmed_transferred = u64::try_from(confirmed)?
        .checked_mul(TRANSFER_LAMPORTS)
        .ok_or_else(|| io::Error::other("confirmed transfer total overflow"))?;
    let expected_debit = confirmed_transferred
        .checked_add(total_fees)
        .ok_or_else(|| io::Error::other("expected debit overflow"))?;
    let global_budget_ceiling = u64::try_from(transaction_count)?
        .checked_mul(TRANSFER_LAMPORTS + MAX_FEE_LAMPORTS)
        .ok_or_else(|| io::Error::other("global budget overflow"))?;
    assert!(payer_debit <= global_budget_ceiling);

    store.transition_run(run_id, "active", "completed", Utc::now())?;
    store.verify_integrity()?;

    let peak_workers = first_summary.peak_workers.max(second_summary.peak_workers);
    assert!(peak_workers <= max_concurrency);
    let wall_time_ms = u64::try_from(wall_started.elapsed().as_millis())?;
    let submission_ms = u64::try_from((first_elapsed + second_elapsed).as_millis())?;
    let database_bytes = fs::metadata(&database_path)?.len();
    let confirmed_per_second = if submission_ms == 0 {
        0.0
    } else {
        #[allow(
            clippy::cast_precision_loss,
            reason = "throughput is a reported rate, not an exact quantity"
        )]
        let rate = confirmed as f64 * 1000.0 / submission_ms as f64;
        (rate * 1000.0).round() / 1000.0
    };

    let evidence = json!({
        "schema_version": 2,
        "scenario": "bounded_devnet_native_transfer_run",
        "network": {
            "cluster": PublicCluster::Devnet.as_str(),
            "rpc": endpoint.for_evidence(),
            "genesis_hash": cluster.genesis_hash,
            "solana_core": cluster.solana_core,
            "feature_set": cluster.feature_set,
            "program_id": SYSTEM_PROGRAM_ID,
            "explorer_signature_url": "https://explorer.solana.com/tx/<signature>?cluster=devnet",
            "explorer_address_url": "https://explorer.solana.com/address/<address>?cluster=devnet",
        },
        "payer": payer_address,
        "started_at": started_at.to_rfc3339(),
        "finished_at": Utc::now().to_rfc3339(),
        "wall_time_ms": wall_time_ms,
        "submission_wall_time_ms": submission_ms,
        "confirmed_per_second": confirmed_per_second,
        "transaction_count": transaction_count,
        "logical_action_count": logical_ids.len(),
        "max_concurrency": max_concurrency,
        "peak_worker_count": peak_workers,
        "confirmed_action_count": confirmed,
        "failed_action_count": failed,
        "expired_action_count": expired,
        "rejected_action_count": rejected,
        "unconfirmed_action_count": failed + expired + rejected,
        "unconfirmed_causes": unconfirmed_causes,
        "duplicate_logical_intents": 0,
        "duplicate_enqueue_attempts": transaction_count,
        "idempotent_duplicate_enqueue_rejections": idempotent_enqueue_rejections,
        "duplicate_signatures": duplicate_signatures,
        "budget_violations": budget_violations,
        "unresolved_submitted": unresolved_submitted,
        "unresolved_unknown": unresolved_unknown,
        "response_loss_injections": 1,
        "submit_visibility_barriers": first_batch_size,
        "ambiguous_outcomes_before_reconstruction": first_summary.unknown,
        "ambiguous_outcomes_after_reconstruction": second_summary.unknown,
        "reconciled_without_resend": reconciled_confirmed + reconciled_failed + reconciled_expired,
        "reconciled_to_confirmed": reconciled_confirmed,
        "reconciled_to_failed": reconciled_failed,
        "reconciled_to_expired": reconciled_expired,
        "reconciliation_rounds": reconciliation_rounds,
        "in_process_component_reconstructions": 1,
        "component_reconstruction": {
            "os_process_restarts": 0,
            "runtime_engine_rebuilt": true,
            "sqlite_database_reopened": true,
            "signer_reloaded": true,
            "gateway_reused": true,
        },
        "journal_event_count": journal_events,
        "database_bytes": database_bytes,
        "exact_source_debit_count": exact_source_debits,
        "exact_destination_credit_count": exact_destination_credits,
        "distinct_confirmed_slots": slots.len(),
        "first_confirmed_slot": slots.iter().next(),
        "last_confirmed_slot": slots.iter().next_back(),
        "epoch_before": epoch_before.epoch,
        "epoch_after": epoch_after.epoch,
        "absolute_slot_before": epoch_before.absolute_slot,
        "absolute_slot_after": epoch_after.absolute_slot,
        "state_delta": {
            "payer_before_lamports": payer_before,
            "payer_after_lamports": payer_after,
            "payer_debit_lamports": payer_debit,
            "confirmed_transferred_lamports": confirmed_transferred,
            "fees_lamports": total_fees,
            "expected_debit_lamports": expected_debit,
            "payer_equation_met": payer_debit == expected_debit,
            "each_destination_delta_lamports": TRANSFER_LAMPORTS,
        },
        "signature_order": "planned_sequence",
        "signatures": transactions,
    });
    write_json(&evidence_path, &evidence)?;
    println!("COOKER_DEVNET_EVIDENCE_PATH={}", evidence_path.display());
    println!(
        "COOKER_DEVNET_SUMMARY confirmed={confirmed} failed={failed} expired={expired} \
         rejected={rejected} wall_ms={wall_time_ms} confirmed_per_second={confirmed_per_second}"
    );
    Ok(())
}

/// Classify why an action never reached a confirmation, from its immutable event journal.
///
/// The distinction retained here is between explicit RPC-edge rejection and a persisted
/// signature that remained absent through expiry. Absence does not prove where the send was
/// lost, so the two outcomes are reported separately.
fn classify_unconfirmed_cause(events: &[cooker_store::ActionEventRecord]) -> &'static str {
    let mut cause = "unclassified";
    for event in events {
        let Some(detail) = event.detail.as_deref() else {
            continue;
        };
        if detail.contains("injected devnet send-response loss") {
            cause = "injected_response_loss";
        } else if detail.contains("sendTransaction") && detail.contains("429") {
            cause = "rpc_edge_rate_limited_send";
        } else if detail.contains("429") {
            cause = "rpc_edge_rate_limited_read";
        } else if detail.contains("blockhash validity window elapsed") {
            // Keep an earlier, more specific cause: expiry is the outcome, not the reason.
            if cause == "unclassified" {
                cause = "not_included_before_blockhash_expiry";
            }
        }
    }
    cause
}

fn run_scoped_destination(run_id: RunId, sequence: u64) -> Pubkey {
    let mut bytes = [0_u8; 32];
    bytes[..8].copy_from_slice(b"cooker-d");
    bytes[8..24].copy_from_slice(run_id.0.as_bytes());
    bytes[24..].copy_from_slice(&sequence.to_le_bytes());
    Pubkey::new_from_array(bytes)
}
