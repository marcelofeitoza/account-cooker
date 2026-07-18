//! Durable compressed soak against real local Surfpool transactions.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Path, PathBuf},
    process::Command,
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use chrono::{TimeDelta, Utc};
use cooker_core::{
    ActionAdapter, ActionId, ActionKind, ActionPayload, ActionState, AdapterContext, AgentId,
    BudgetConfig, ChainGateway, ChainReceipt, Clock, CookerError, PlannedAction, Policy,
    PolicyConfig, RunId, SafetyPolicy, SimulationReceipt, StateStore, SystemClock,
};
use cooker_runtime::{
    ExecutionCheckpoint, ExecutionResult, FaultInjector, NoFaults, RuntimeEngine, RuntimeSettings,
};
use cooker_solana::{LocalKeypair, NativeTransferAdapter, SurfpoolGateway, SurfpoolRpcUrl};
use cooker_store::{RunRegistration, Store, StoreIdentity};
use serde_json::json;
use solana_pubkey::Pubkey;
use solana_signature::Signature;
use tokio::sync::watch;

const DEFAULT_TRANSACTION_COUNT: usize = 1_000;
const TRANSFER_LAMPORTS: u64 = 1_000_000;
const MAX_FEE_LAMPORTS: u64 = 10_000;
const MODEL_VERSION: &str = "surfpool-compressed-soak-v1";

#[derive(Debug, Default)]
struct LoseOneSendResponse {
    triggered: AtomicBool,
}

impl FaultInjector for LoseOneSendResponse {
    fn check(&self, checkpoint: ExecutionCheckpoint) -> Result<(), CookerError> {
        if checkpoint == ExecutionCheckpoint::AfterSendResponseLost
            && !self.triggered.swap(true, Ordering::SeqCst)
        {
            return Err(CookerError::Chain(
                "injected Surfpool send-response loss".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug)]
struct ConfirmSubmissionGateway {
    inner: Arc<SurfpoolGateway>,
}

impl ConfirmSubmissionGateway {
    fn new(inner: Arc<SurfpoolGateway>) -> Self {
        Self { inner }
    }
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
            if let Some(record) = self.inner.transaction(&parsed).await? {
                if let Some(error) = record.error {
                    return Err(CookerError::Chain(format!(
                        "soak submission failed before response loss: {error}"
                    )));
                }
                break;
            }
            if started.elapsed() >= Duration::from_secs(20) {
                return Err(CookerError::Chain(
                    "soak submission was not observable before response loss".to_owned(),
                ));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
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
#[ignore = "executes at least 1,000 locally signed transactions against pinned Surfpool"]
#[allow(
    clippy::too_many_lines,
    reason = "the live soak keeps setup, restart, reconciliation, and evidence assertions visible"
)]
async fn compressed_soak_restarts_and_reconciles_without_duplicate_intents()
-> Result<(), Box<dyn std::error::Error>> {
    let wall_started = Instant::now();
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let transaction_count = env_usize("COOKER_SOAK_TRANSACTIONS", DEFAULT_TRANSACTION_COUNT)?;
    if transaction_count < DEFAULT_TRANSACTION_COUNT
        && std::env::var_os("COOKER_SOAK_ALLOW_SHORT").is_none()
    {
        return Err(io::Error::other(format!(
            "acceptance soak requires at least {DEFAULT_TRANSACTION_COUNT} transactions"
        ))
        .into());
    }
    let max_concurrency = env_usize("COOKER_SOAK_CONCURRENCY", 1)?;
    if max_concurrency > 64 {
        return Err(io::Error::other("soak concurrency is limited to 64").into());
    }
    let evidence_path = std::env::var_os("COOKER_SOAK_EVIDENCE").map_or_else(
        || project_root.join("evidence/tmp/surfpool-soak.json"),
        PathBuf::from,
    );
    let database_path = std::env::var_os("COOKER_SOAK_DATABASE").map_or_else(
        || project_root.join("evidence/tmp/surfpool-soak.sqlite"),
        PathBuf::from,
    );
    if database_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("soak database already exists: {}", database_path.display()),
        )
        .into());
    }
    if let Some(parent) = database_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let rpc_url =
        std::env::var("COOKER_RPC_URL").unwrap_or_else(|_| "http://127.0.0.1:8899".to_owned());
    let signer_path = std::env::var_os("COOKER_SIGNER_PATH").map_or_else(
        || project_root.join(".surfpool/keys/funder.json"),
        PathBuf::from,
    );
    let endpoint: SurfpoolRpcUrl = rpc_url.parse()?;
    let gateway = Arc::new(SurfpoolGateway::connect(endpoint.clone()).await?);
    let signer = Arc::new(LocalKeypair::load(&project_root, &signer_path)?);
    let funder = signer.pubkey();
    let funder_before = gateway.balance(&funder).await?;
    let surfpool_version = gateway.identity().surfnet_version.clone();

    let run_id = RunId::new();
    let scheduled_at = Utc::now();
    let actions = planned_actions(run_id, scheduled_at, transaction_count);
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
        StoreIdentity::surfpool("noise-dev-soak")?,
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
            "rpc": "loopback-surfpool",
        }),
        config_hash: "c001c0de".to_owned(),
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

    let loss = Arc::new(LoseOneSendResponse::default());
    let first_runtime = runtime(
        Arc::clone(&store),
        Arc::clone(&gateway),
        Arc::clone(&signer),
        &destinations,
        max_concurrency,
        loss.clone(),
        true,
    )?;
    let first_batch_size = (transaction_count / 2).max(1);
    let first_claimed = store.claim_due_actions(
        "soak-before-restart",
        Utc::now(),
        Utc::now() + TimeDelta::minutes(30),
        first_batch_size,
    )?;
    assert_eq!(first_claimed.len(), first_batch_size);
    let (_first_stop, first_stop_rx) = watch::channel(false);
    let first_summary = Arc::clone(&first_runtime)
        .run_claimed(first_claimed, first_stop_rx)
        .await;
    assert!(first_summary.errors.is_empty(), "{first_summary:?}");
    assert_eq!(first_summary.unknown, 1);
    assert_eq!(
        first_summary.confirmed + first_summary.unknown,
        first_batch_size
    );
    assert!(loss.triggered.load(Ordering::SeqCst));

    // Deliberately discard every process-local component and reconstruct it from the WAL file.
    drop(first_runtime);
    drop(store);
    drop(signer);
    drop(gateway);

    let surfpool_restarts = if std::env::var_os("COOKER_SOAK_RESTART_SURFPOOL").is_some() {
        restart_surfpool(&project_root)?;
        1
    } else {
        0
    };

    let gateway = Arc::new(SurfpoolGateway::connect(endpoint).await?);
    let signer = Arc::new(LocalKeypair::load(&project_root, &signer_path)?);
    let store = Arc::new(Store::open(
        &database_path,
        StoreIdentity::surfpool("noise-dev-soak")?,
    )?);
    store.verify_integrity()?;
    let second_runtime = runtime(
        Arc::clone(&store),
        Arc::clone(&gateway),
        Arc::clone(&signer),
        &destinations,
        max_concurrency,
        Arc::new(NoFaults),
        false,
    )?;
    let second_claimed = store.claim_due_actions(
        "soak-after-restart",
        Utc::now(),
        Utc::now() + TimeDelta::minutes(30),
        transaction_count,
    )?;
    assert_eq!(second_claimed.len(), transaction_count - first_batch_size);
    let (_second_stop, second_stop_rx) = watch::channel(false);
    let second_summary = Arc::clone(&second_runtime)
        .run_claimed(second_claimed, second_stop_rx)
        .await;
    assert!(second_summary.errors.is_empty(), "{second_summary:?}");
    assert_eq!(
        second_summary.confirmed,
        transaction_count - first_batch_size
    );
    assert_eq!(second_summary.unknown, 0);
    assert_eq!(second_summary.rejected, 0);
    assert_eq!(second_summary.failed, 0);

    let recovery_leases = store.claim_reconciliation_candidates(
        "soak-reconciler",
        Utc::now(),
        Utc::now() + TimeDelta::minutes(30),
        transaction_count,
    )?;
    assert_eq!(recovery_leases.len(), 1);
    let mut reconciled = 0_usize;
    for lease in recovery_leases {
        match second_runtime.reconcile(&lease).await? {
            ExecutionResult::Confirmed { .. } => reconciled += 1,
            other => {
                return Err(io::Error::other(format!(
                    "response-loss reconciliation was not confirmed: {other:?}"
                ))
                .into());
            }
        }
    }
    assert_eq!(reconciled, 1);

    let remaining_reconciliation = store.claim_reconciliation_candidates(
        "soak-final-reconciler",
        Utc::now(),
        Utc::now() + TimeDelta::minutes(5),
        transaction_count,
    )?;
    assert!(remaining_reconciliation.is_empty());
    let remaining_due = store.claim_due_actions(
        "soak-final-due-check",
        Utc::now(),
        Utc::now() + TimeDelta::minutes(5),
        transaction_count,
    )?;
    assert!(remaining_due.is_empty());

    let mut confirmed = 0_usize;
    let mut unresolved_submitted = 0_usize;
    let mut unresolved_unknown = 0_usize;
    let mut duplicate_signatures = 0_usize;
    let mut budget_violations = 0_usize;
    let mut signatures = BTreeSet::new();
    let mut signature_samples = Vec::new();
    let mut total_fees = 0_u64;
    let mut journal_events = 0_usize;
    for action in &actions {
        match store.action_state(&action.id)? {
            ActionState::Confirmed => confirmed += 1,
            ActionState::Submitted => unresolved_submitted += 1,
            ActionState::Unknown => unresolved_unknown += 1,
            state => {
                return Err(io::Error::other(format!(
                    "soak action {} ended in {state:?}",
                    action.id
                ))
                .into());
            }
        }
        let recovery = store.recovery_record(&action.id, Utc::now())?;
        let submission = recovery
            .submission
            .ok_or_else(|| io::Error::other("confirmed action omitted its submission journal"))?;
        if !signatures.insert(submission.signature.clone()) {
            duplicate_signatures += 1;
        }
        if signature_samples.len() < 12 {
            signature_samples.push(sanitize_signature(&submission.signature));
        }
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
        if fee > MAX_FEE_LAMPORTS {
            budget_violations += 1;
        }
        total_fees = total_fees
            .checked_add(fee)
            .ok_or_else(|| io::Error::other("total fee overflow"))?;
        let ActionPayload::NativeTransfer { destination, .. } = &action.payload else {
            return Err(io::Error::other("soak action was not a native transfer").into());
        };
        let destination_observation = receipt
            .observations
            .iter()
            .find(|observation| {
                observation.kind == "native_destination_balance_delta"
                    && observation.account == *destination
            })
            .ok_or_else(|| io::Error::other("receipt omitted destination state delta"))?;
        if destination_observation.expected_delta != Some(i128::from(TRANSFER_LAMPORTS))
            || destination_observation.attributes.get("actual_delta")
                != Some(&TRANSFER_LAMPORTS.to_string())
            || destination_observation
                .attributes
                .get("met")
                .map(String::as_str)
                != Some("true")
        {
            return Err(io::Error::other("destination state delta was not exact").into());
        }
        let usage = store.budget_usage(action.agent_id, Utc::now())?;
        let action_ceiling = TRANSFER_LAMPORTS + MAX_FEE_LAMPORTS;
        if usage.spent_today_lamports > action_ceiling
            || usage.spent_lifetime_lamports > action_ceiling
        {
            budget_violations += 1;
        }
        journal_events += recovery.events.len();
    }
    assert_eq!(confirmed, transaction_count);
    assert_eq!(signatures.len(), transaction_count);
    assert_eq!(duplicate_signatures, 0);
    assert_eq!(budget_violations, 0);
    assert_eq!(unresolved_submitted, 0);
    assert_eq!(unresolved_unknown, 0);

    for destination in &destinations {
        assert_eq!(
            gateway.native_balance(destination).await?,
            TRANSFER_LAMPORTS
        );
    }
    let funder_after = gateway.balance(&funder).await?;
    let total_transferred = u64::try_from(transaction_count)?
        .checked_mul(TRANSFER_LAMPORTS)
        .ok_or_else(|| io::Error::other("total transfer overflow"))?;
    let payer_debit = funder_before
        .checked_sub(funder_after)
        .ok_or_else(|| io::Error::other("soak payer balance unexpectedly increased"))?;
    assert_eq!(payer_debit, total_transferred + total_fees);
    let global_budget_ceiling = u64::try_from(transaction_count)?
        .checked_mul(TRANSFER_LAMPORTS + MAX_FEE_LAMPORTS)
        .ok_or_else(|| io::Error::other("global budget overflow"))?;
    assert!(payer_debit <= global_budget_ceiling);
    store.transition_run(run_id, "active", "completed", Utc::now())?;
    store.verify_integrity()?;

    let peak_workers = first_summary
        .peak_workers
        .max(second_summary.peak_workers)
        .max(usize::from(reconciled > 0));
    assert!(peak_workers <= max_concurrency);
    let failures = first_summary.failed
        + first_summary.rejected
        + first_summary.errors.len()
        + second_summary.failed
        + second_summary.rejected
        + second_summary.errors.len();
    assert_eq!(failures, 0);
    let wall_time_ms = u64::try_from(wall_started.elapsed().as_millis())?;
    let database_bytes = fs::metadata(&database_path)?.len();
    let evidence = json!({
        "schema_version": 1,
        "scenario": "compressed_surfpool_native_transfer_soak",
        "surfpool_version": surfpool_version,
        "rpc": "http://127.0.0.1:<local>",
        "wall_time_ms": wall_time_ms,
        "transaction_count": transaction_count,
        "logical_action_count": logical_ids.len(),
        "confirmed_action_count": confirmed,
        "failures": failures,
        "duplicate_logical_intents": 0,
        "idempotent_duplicate_enqueue_rejections": idempotent_enqueue_rejections,
        "duplicate_signatures": duplicate_signatures,
        "budget_violations": budget_violations,
        "unresolved_submitted": unresolved_submitted,
        "unresolved_unknown": unresolved_unknown,
        "response_loss_injections": 1,
        "submit_visibility_barriers": first_batch_size,
        "response_loss_reconciled_without_resend": reconciled,
        "runtime_stack_restarts": 1,
        "surfpool_process_restarts": surfpool_restarts,
        "max_concurrency": max_concurrency,
        "peak_worker_count": peak_workers,
        "journal_event_count": journal_events,
        "database_bytes": database_bytes,
        "state_delta": {
            "payer_before_lamports": funder_before,
            "payer_after_lamports": funder_after,
            "payer_debit_lamports": payer_debit,
            "transferred_lamports": total_transferred,
            "fees_lamports": total_fees,
            "destination_accounts": transaction_count,
            "each_destination_delta_lamports": TRANSFER_LAMPORTS,
        },
        "sanitized_signature_samples": signature_samples,
        "database_file": database_path.file_name().map(|value| value.to_string_lossy()),
    });
    write_json(&evidence_path, &evidence)?;
    println!("COOKER_SOAK_EVIDENCE={evidence}");
    Ok(())
}

fn planned_actions(
    run_id: RunId,
    scheduled_at: chrono::DateTime<Utc>,
    count: usize,
) -> Vec<PlannedAction> {
    (0..count)
        .map(|index| {
            let agent_id = AgentId::new();
            let sequence = u64::try_from(index).unwrap_or(u64::MAX);
            let destination = deterministic_destination(sequence);
            PlannedAction {
                id: ActionId::derive(run_id, agent_id, sequence, MODEL_VERSION),
                run_id,
                agent_id,
                sequence,
                model_version: MODEL_VERSION.to_owned(),
                scheduled_at,
                payload: ActionPayload::NativeTransfer {
                    destination: destination.to_string(),
                    lamports: TRANSFER_LAMPORTS,
                },
                max_fee_lamports: MAX_FEE_LAMPORTS,
                max_account_creation_lamports: 0,
                created_at: scheduled_at,
            }
        })
        .collect()
}

fn deterministic_destination(sequence: u64) -> Pubkey {
    let mut bytes = [0_u8; 32];
    bytes[..16].copy_from_slice(b"noise-soak-v1!!!");
    bytes[24..].copy_from_slice(&sequence.to_le_bytes());
    Pubkey::new_from_array(bytes)
}

fn runtime(
    store: Arc<Store>,
    gateway: Arc<SurfpoolGateway>,
    signer: Arc<LocalKeypair>,
    destinations: &BTreeSet<String>,
    max_concurrency: usize,
    faults: Arc<dyn FaultInjector>,
    confirm_first_submission: bool,
) -> Result<Arc<RuntimeEngine>, CookerError> {
    let total_budget = u64::try_from(destinations.len())
        .unwrap_or(u64::MAX)
        .saturating_mul(TRANSFER_LAMPORTS + MAX_FEE_LAMPORTS);
    let policy: Arc<dyn Policy> = Arc::new(SafetyPolicy::new(PolicyConfig {
        budgets: BudgetConfig {
            daily_lamports: total_budget,
            lifetime_lamports: total_budget,
            reserve_lamports: 100_000_000,
            max_fee_lamports: MAX_FEE_LAMPORTS,
            max_account_creation_lamports: 0,
        },
        allowed_actions: [ActionKind::NativeTransfer].into_iter().collect(),
        max_action_amounts: BTreeMap::from([(ActionKind::NativeTransfer, TRANSFER_LAMPORTS)]),
        allowed_destinations: destinations.clone(),
        allowed_mints: BTreeSet::new(),
        native_input_mints: BTreeSet::new(),
        max_slippage_bps: 0,
    })?);
    let context = AdapterContext {
        rpc_url: gateway.endpoint().as_url().clone(),
        signer: signer.pubkey().to_string(),
        confirmation_timeout: Duration::from_secs(20),
    };
    let adapter: Arc<dyn ActionAdapter> =
        Arc::new(NativeTransferAdapter::new(Arc::clone(&gateway), signer));
    let store_trait: Arc<dyn StateStore> = store;
    let gateway_trait: Arc<dyn ChainGateway> = if confirm_first_submission {
        Arc::new(ConfirmSubmissionGateway::new(gateway))
    } else {
        gateway
    };
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    Ok(Arc::new(RuntimeEngine::new(
        store_trait,
        gateway_trait,
        policy,
        [(ActionKind::NativeTransfer, adapter)],
        RuntimeSettings {
            adapter_context: context,
            max_concurrency,
        },
        faults,
        clock,
    )?))
}

fn env_usize(name: &str, default: usize) -> Result<usize, Box<dyn std::error::Error>> {
    std::env::var(name).map_or(Ok(default), |value| {
        let parsed = value.parse::<usize>()?;
        if parsed == 0 {
            return Err(io::Error::other(format!("{name} must be positive")).into());
        }
        Ok(parsed)
    })
}

fn sanitize_signature(signature: &str) -> String {
    if signature.len() <= 20 {
        return signature.to_owned();
    }
    format!(
        "{}...{}",
        &signature[..10],
        &signature[signature.len() - 10..]
    )
}

fn restart_surfpool(project_root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    for script in [
        "surfpool-stop.sh",
        "surfpool-start.sh",
        "surfpool-doctor.sh",
    ] {
        let status = Command::new(project_root.join("scripts").join(script))
            .current_dir(project_root)
            .status()?;
        if !status.success() {
            return Err(io::Error::other(format!(
                "{script} failed during injected Surfpool restart with {status}"
            ))
            .into());
        }
    }
    Ok(())
}

fn write_json(path: &Path, value: &serde_json::Value) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("json.tmp");
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    fs::write(&temporary, bytes)?;
    fs::rename(temporary, path)?;
    Ok(())
}
