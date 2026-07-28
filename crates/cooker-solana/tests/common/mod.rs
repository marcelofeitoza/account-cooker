//! Scaffolding shared by the chain-facing integration tests.
//!
//! Every item here was copied into two or more test binaries before it was single-sourced.
//! Nothing in this module asserts anything: it only builds the objects a test needs, so each
//! test file still contains its own claims and nothing else.

#![allow(
    dead_code,
    reason = "each test binary links a different subset of the shared scaffolding"
)]

use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fs, io,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use chrono::{DateTime, Utc};
use cooker_core::{
    ActionAdapter, ActionId, ActionKind, ActionPayload, AdapterContext, AgentId, BudgetConfig,
    ChainGateway, Clock, CookerError, PlannedAction, Policy, PolicyConfig, RunId, SafetyPolicy,
    StateStore, SystemClock,
};
use cooker_runtime::{ExecutionCheckpoint, FaultInjector, RuntimeEngine, RuntimeSettings};
use cooker_solana::{
    LocalKeypair, NativeTransferAdapter, RpcEndpoint, SolanaGateway, TransactionRecord,
};
use cooker_store::Store;
use solana_pubkey::Pubkey;

/// Loopback endpoint the local harness publishes when no override is set.
pub const DEFAULT_LOCAL_RPC_URL: &str = "http://127.0.0.1:8899";
/// Confirmation ceiling every loopback test uses.
pub const DEFAULT_CONFIRMATION_TIMEOUT: Duration = Duration::from_secs(20);

/// Repository root relative to the crate being tested.
pub fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Gateway, signer, and adapter context for one local Surfpool test.
#[derive(Debug)]
pub struct LiveContext {
    /// Repository root the signer was resolved against.
    pub project_root: PathBuf,
    /// Path of the harness-funded local keypair.
    pub signer_path: PathBuf,
    /// Identity-verified gateway.
    pub gateway: Arc<SolanaGateway>,
    /// Loaded local signer.
    pub signer: Arc<LocalKeypair>,
    /// Adapter context bound to the same endpoint and signer.
    pub context: AdapterContext,
}

/// Connect to the local harness and load its funded signer.
pub async fn live_context() -> Result<LiveContext, Box<dyn Error>> {
    live_context_in(project_root()).await
}

/// Connect to the local harness using an explicit repository root.
pub async fn live_context_in(project_root: PathBuf) -> Result<LiveContext, Box<dyn Error>> {
    let rpc_url =
        std::env::var("COOKER_RPC_URL").unwrap_or_else(|_| DEFAULT_LOCAL_RPC_URL.to_owned());
    let signer_path = std::env::var_os("COOKER_SIGNER_PATH").map_or_else(
        || project_root.join(".surfpool/keys/funder.json"),
        PathBuf::from,
    );
    let endpoint: RpcEndpoint = rpc_url.parse()?;
    let gateway = Arc::new(SolanaGateway::connect(endpoint).await?);
    let signer = Arc::new(LocalKeypair::load(&project_root, &signer_path)?);
    let context = AdapterContext {
        rpc_url: gateway.endpoint().as_url().clone(),
        signer: signer.pubkey().to_string(),
        confirmation_timeout: DEFAULT_CONFIRMATION_TIMEOUT,
    };
    Ok(LiveContext {
        project_root,
        signer_path,
        gateway,
        signer,
        context,
    })
}

/// Shorten a signature so evidence files never carry a full one.
pub fn sanitize_signature(signature: &impl ToString) -> String {
    let signature = signature.to_string();
    if signature.len() <= 20 {
        return signature;
    }
    format!(
        "{}...{}",
        &signature[..10],
        &signature[signature.len() - 10..]
    )
}

/// Write pretty JSON through a temporary file so a reader never sees a partial evidence file.
pub fn write_json(path: &Path, value: &serde_json::Value) -> Result<(), Box<dyn Error>> {
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

/// Read a positive `usize` override from the environment.
pub fn env_usize(name: &str, default: usize) -> Result<usize, Box<dyn Error>> {
    std::env::var(name).map_or(Ok(default), |value| {
        let parsed = value.parse::<usize>()?;
        if parsed == 0 {
            return Err(io::Error::other(format!("{name} must be positive")).into());
        }
        Ok(parsed)
    })
}

/// Read a positive `u64` override from the environment.
pub fn env_u64(name: &str, default: u64) -> Result<u64, Box<dyn Error>> {
    std::env::var(name).map_or(Ok(default), |value| {
        let parsed = value.parse::<u64>()?;
        if parsed == 0 {
            return Err(io::Error::other(format!("{name} must be positive")).into());
        }
        Ok(parsed)
    })
}

/// Classify why an action never reached a confirmation, from its immutable event journal.
///
/// The distinction retained here is between explicit RPC-edge rejection and a persisted
/// signature that remained absent through expiry. Absence does not prove where the send was
/// lost, so the two outcomes are reported separately.
pub fn classify_unconfirmed_cause(events: &[cooker_store::ActionEventRecord]) -> &'static str {
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

/// Advance the local chain until it passes `target`, returning how many slots it took.
pub async fn advance_past_block_height(
    gateway: &SolanaGateway,
    target: u64,
    max_advance: u64,
) -> Result<u64, Box<dyn Error>> {
    let mut advanced = 0_u64;
    while gateway.block_height().await? <= target {
        if advanced >= max_advance {
            return Err(io::Error::other(format!(
                "local chain did not exceed block height {target} within {max_advance} slots"
            ))
            .into());
        }
        let current = gateway.epoch_info().await?;
        gateway
            .time_travel_to_slot(
                current
                    .absolute_slot
                    .checked_add(1)
                    .ok_or_else(|| io::Error::other("absolute slot overflowed"))?,
            )
            .await?;
        advanced += 1;
    }
    Ok(advanced)
}

/// Read one account's pre and post native balance out of transaction metadata.
pub fn transaction_native_balances(
    record: &TransactionRecord,
    address: &Pubkey,
    label: &str,
) -> Result<(u64, u64), io::Error> {
    let index = record
        .account_keys
        .iter()
        .position(|candidate| candidate == address)
        .ok_or_else(|| io::Error::other(format!("{label} missing from transaction accounts")))?;
    let before = record
        .pre_balances
        .get(index)
        .copied()
        .ok_or_else(|| io::Error::other(format!("{label} pre-balance missing")))?;
    let after = record
        .post_balances
        .get(index)
        .copied()
        .ok_or_else(|| io::Error::other(format!("{label} post-balance missing")))?;
    Ok((before, after))
}

/// Fault injector that drops exactly one send response, once.
#[derive(Debug)]
pub struct LoseOneSendResponse {
    triggered: AtomicBool,
    message: &'static str,
}

impl LoseOneSendResponse {
    /// Build an injector that reports `message` when it fires.
    pub const fn new(message: &'static str) -> Self {
        Self {
            triggered: AtomicBool::new(false),
            message,
        }
    }

    /// Whether the single injection has already fired.
    pub fn triggered(&self) -> bool {
        self.triggered.load(Ordering::SeqCst)
    }
}

impl FaultInjector for LoseOneSendResponse {
    fn check(&self, checkpoint: ExecutionCheckpoint) -> Result<(), CookerError> {
        if checkpoint == ExecutionCheckpoint::AfterSendResponseLost
            && !self.triggered.swap(true, Ordering::SeqCst)
        {
            return Err(CookerError::Chain(self.message.to_owned()));
        }
        Ok(())
    }
}

/// Per-run quantities a bounded native-transfer soak is built from.
#[derive(Clone, Copy, Debug)]
pub struct NativeSoakParams {
    /// Model version stamped onto every action identifier.
    pub model_version: &'static str,
    /// Lamports each transfer moves.
    pub transfer_lamports: u64,
    /// Per-transaction fee ceiling.
    pub max_fee_lamports: u64,
    /// Lamports the payer must keep untouched.
    pub reserve_lamports: u64,
    /// Confirmation ceiling for the adapter context.
    pub confirmation_timeout: Duration,
    /// Worker ceiling the runtime engine is allowed to reach.
    pub max_concurrency: usize,
}

/// Build the run's planned native transfers, one per unique destination.
pub fn planned_native_actions(
    run_id: RunId,
    scheduled_at: DateTime<Utc>,
    count: usize,
    params: NativeSoakParams,
    destination: impl Fn(u64) -> Pubkey,
) -> Vec<PlannedAction> {
    (0..count)
        .map(|index| {
            let agent_id = AgentId::new();
            let sequence = u64::try_from(index).unwrap_or(u64::MAX);
            PlannedAction {
                id: ActionId::derive(run_id, agent_id, sequence, params.model_version),
                run_id,
                agent_id,
                sequence,
                model_version: params.model_version.to_owned(),
                scheduled_at,
                payload: ActionPayload::NativeTransfer {
                    destination: destination(sequence).to_string(),
                    lamports: params.transfer_lamports,
                },
                max_fee_lamports: params.max_fee_lamports,
                max_account_creation_lamports: 0,
                created_at: scheduled_at,
            }
        })
        .collect()
}

/// Build the runtime engine a bounded native-transfer soak drives.
///
/// `chain` is taken separately from `gateway` so a test can wrap the real gateway in its own
/// observing decorator without this helper knowing anything about that decorator.
pub fn native_transfer_runtime(
    store: Arc<Store>,
    gateway: &Arc<SolanaGateway>,
    chain: Arc<dyn ChainGateway>,
    signer: Arc<LocalKeypair>,
    destinations: &BTreeSet<String>,
    params: NativeSoakParams,
    faults: Arc<dyn FaultInjector>,
) -> Result<Arc<RuntimeEngine>, CookerError> {
    let total_budget = u64::try_from(destinations.len())
        .unwrap_or(u64::MAX)
        .saturating_mul(params.transfer_lamports + params.max_fee_lamports);
    let policy: Arc<dyn Policy> = Arc::new(SafetyPolicy::new(PolicyConfig {
        budgets: BudgetConfig {
            daily_lamports: total_budget,
            lifetime_lamports: total_budget,
            reserve_lamports: params.reserve_lamports,
            max_fee_lamports: params.max_fee_lamports,
            max_account_creation_lamports: 0,
        },
        allowed_actions: [ActionKind::NativeTransfer].into_iter().collect(),
        max_action_amounts: BTreeMap::from([(
            ActionKind::NativeTransfer,
            params.transfer_lamports,
        )]),
        allowed_destinations: destinations.clone(),
        allowed_mints: BTreeSet::new(),
        native_input_mints: BTreeSet::new(),
        max_slippage_bps: 0,
    })?);
    let context = AdapterContext {
        rpc_url: gateway.endpoint().as_url().clone(),
        signer: signer.pubkey().to_string(),
        confirmation_timeout: params.confirmation_timeout,
    };
    let adapter: Arc<dyn ActionAdapter> =
        Arc::new(NativeTransferAdapter::new(Arc::clone(gateway), signer));
    let store_trait: Arc<dyn StateStore> = store;
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    Ok(Arc::new(RuntimeEngine::new(
        store_trait,
        chain,
        policy,
        [(ActionKind::NativeTransfer, adapter)],
        RuntimeSettings {
            adapter_context: context,
            max_concurrency: params.max_concurrency,
        },
        faults,
        clock,
    )?))
}
