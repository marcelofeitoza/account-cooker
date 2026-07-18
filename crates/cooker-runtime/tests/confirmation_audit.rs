//! Durable post-confirmation auditing and rollback recovery proofs.

use std::{
    collections::BTreeMap,
    error::Error,
    str,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use chrono::{DateTime, TimeDelta, TimeZone, Utc};
use cooker_core::{
    ActionAdapter, ActionId, ActionKind, ActionPayload, ActionState, AdapterContext, AgentId,
    ChainGateway, ChainReceipt, Clock, ConfirmationStatus, CookerError, PlannedAction, Policy,
    PolicyDecision, PreparedAction, RunId, SimulationReceipt, StateStore, VirtualClock,
};
use cooker_runtime::{FleetRuntime, NoFaults, RuntimeEngine, RuntimeSettings};
use cooker_store::{ConfirmationAuditOutcome, Store, StoreIdentity};
use tempfile::TempDir;
use tokio::sync::watch;
use url::Url;
use uuid::Uuid;

type TestResult = Result<(), Box<dyn Error>>;

const SIGNER_A: &str = "audit-signer-a";
const SIGNER_B: &str = "audit-signer-b";

#[derive(Clone, Copy, Debug)]
enum ObservationMode {
    Confirmed,
    ConfirmedWithoutPostconditions,
    Processed,
    Missing,
    Failed,
}

#[derive(Debug)]
struct Probe {
    mode: Mutex<ObservationMode>,
    submissions: AtomicUsize,
    observations: Mutex<Vec<String>>,
    block_height: AtomicU64,
}

impl Probe {
    fn new() -> Self {
        Self {
            mode: Mutex::new(ObservationMode::Confirmed),
            submissions: AtomicUsize::new(0),
            observations: Mutex::new(Vec::new()),
            block_height: AtomicU64::new(1),
        }
    }

    fn set_mode(&self, mode: ObservationMode) -> Result<(), CookerError> {
        *self
            .mode
            .lock()
            .map_err(|_| CookerError::Chain("observation mode lock poisoned".to_owned()))? = mode;
        Ok(())
    }

    fn mode(&self) -> Result<ObservationMode, CookerError> {
        self.mode
            .lock()
            .map_err(|_| CookerError::Chain("observation mode lock poisoned".to_owned()))
            .map(|mode| *mode)
    }

    fn record_observation(&self, signer: &str) -> Result<(), CookerError> {
        self.observations
            .lock()
            .map_err(|_| CookerError::Chain("observation log lock poisoned".to_owned()))?
            .push(signer.to_owned());
        Ok(())
    }

    fn observations(&self) -> Result<Vec<String>, CookerError> {
        self.observations
            .lock()
            .map_err(|_| CookerError::Chain("observation log lock poisoned".to_owned()))
            .map(|values| values.clone())
    }
}

#[derive(Debug)]
struct ProbeGateway {
    probe: Arc<Probe>,
}

#[async_trait]
impl ChainGateway for ProbeGateway {
    async fn simulate(&self, _transaction: &[u8]) -> Result<SimulationReceipt, CookerError> {
        Ok(SimulationReceipt {
            succeeded: true,
            units_consumed: Some(400),
            logs: vec!["local simulation".to_owned()],
            error: None,
        })
    }

    async fn submit(&self, transaction: &[u8]) -> Result<String, CookerError> {
        self.probe.submissions.fetch_add(1, Ordering::SeqCst);
        str::from_utf8(transaction)
            .map(str::to_owned)
            .map_err(|error| CookerError::Chain(format!("invalid test transaction: {error}")))
    }

    async fn observe(
        &self,
        _action_id: &ActionId,
        _signature: &str,
    ) -> Result<ChainReceipt, CookerError> {
        Err(CookerError::Chain(
            "tests observe through the signer adapter".to_owned(),
        ))
    }

    async fn native_balance(&self, _address: &str) -> Result<u64, CookerError> {
        Ok(10_000_000_000)
    }

    async fn block_height(&self) -> Result<u64, CookerError> {
        Ok(self.probe.block_height.load(Ordering::SeqCst))
    }
}

#[derive(Debug)]
struct ProbeAdapter {
    probe: Arc<Probe>,
    clock: Arc<VirtualClock>,
}

#[async_trait]
impl ActionAdapter for ProbeAdapter {
    fn supports(&self, payload: &ActionPayload) -> bool {
        matches!(payload, ActionPayload::NativeTransfer { .. })
    }

    async fn prepare(
        &self,
        context: &AdapterContext,
        action: &PlannedAction,
    ) -> Result<PreparedAction, CookerError> {
        let signature = format!("signature:{}:{}", context.signer, action.id);
        Ok(PreparedAction {
            action_id: action.id.clone(),
            transaction: signature.as_bytes().to_vec(),
            signature,
            recent_blockhash: "audit-blockhash".to_owned(),
            last_valid_block_height: 100,
            expectations: Vec::new(),
        })
    }

    async fn observe(
        &self,
        context: &AdapterContext,
        action: &PlannedAction,
        prepared: &PreparedAction,
    ) -> Result<ChainReceipt, CookerError> {
        self.probe.record_observation(&context.signer)?;
        let mode = self.probe.mode()?;
        Ok(ChainReceipt {
            action_id: action.id.clone(),
            signature: prepared.signature.clone(),
            status: match mode {
                ObservationMode::Confirmed | ObservationMode::ConfirmedWithoutPostconditions => {
                    ConfirmationStatus::Confirmed
                }
                ObservationMode::Processed => ConfirmationStatus::Processed,
                ObservationMode::Missing => ConfirmationStatus::Missing,
                ObservationMode::Failed => ConfirmationStatus::Failed,
            },
            slot: matches!(
                mode,
                ObservationMode::Confirmed
                    | ObservationMode::ConfirmedWithoutPostconditions
                    | ObservationMode::Processed
            )
            .then_some(42),
            observed_at: self.clock.now(),
            postconditions_met: matches!(mode, ObservationMode::Confirmed),
            observations: Vec::new(),
            error: matches!(mode, ObservationMode::Failed)
                .then(|| "transaction failed after rollback".to_owned()),
        })
    }
}

#[derive(Clone, Copy, Debug)]
struct AllowAll;

impl Policy for AllowAll {
    fn evaluate(
        &self,
        _action: &PlannedAction,
        _signer_balance: u64,
        _spent_today: u64,
        _spent_lifetime: u64,
    ) -> Result<PolicyDecision, CookerError> {
        Ok(PolicyDecision::Allow)
    }
}

struct Harness {
    _directory: TempDir,
    store: Arc<Store>,
    clock: Arc<VirtualClock>,
    probe: Arc<Probe>,
    fleet: Arc<FleetRuntime>,
    action: PlannedAction,
}

#[tokio::test]
async fn successful_audit_is_durable_routed_and_not_immediately_due_again() -> TestResult {
    let harness = confirmed_harness("audit-success", 10).await?;
    harness.clock.advance_by(TimeDelta::hours(1))?;
    let audit_time = harness.clock.now();
    let leases = harness.store.claim_confirmation_audit_candidates(
        "auditor",
        audit_time,
        audit_time + TimeDelta::minutes(1),
        audit_time,
        10,
    )?;
    assert_eq!(leases.len(), 1);
    let submissions_before = harness.probe.submissions.load(Ordering::SeqCst);
    let (_stop_tx, stop_rx) = watch::channel(false);

    let summary = Arc::clone(&harness.fleet)
        .audit_claimed(leases, stop_rx)
        .await;

    assert_eq!(summary.audited, 1);
    assert_eq!(summary.orphaned, 0);
    assert!(summary.errors.is_empty());
    assert_eq!(
        harness.probe.submissions.load(Ordering::SeqCst),
        submissions_before
    );
    assert_eq!(
        harness.probe.observations()?.last().map(String::as_str),
        Some(SIGNER_B)
    );
    let audits = harness.store.confirmation_audits(&harness.action.id)?;
    assert_eq!(audits.len(), 1);
    assert_eq!(audits[0].outcome, ConfirmationAuditOutcome::Verified);
    assert_eq!(audits[0].audited_at, audit_time);
    assert_eq!(
        harness.store.get_action_state(&harness.action.id)?,
        ActionState::Confirmed
    );
    assert!(
        harness
            .store
            .claim_confirmation_audit_candidates(
                "immediate-second-auditor",
                audit_time,
                audit_time + TimeDelta::minutes(1),
                audit_time,
                10,
            )?
            .is_empty()
    );
    Ok(())
}

#[tokio::test]
async fn missing_audit_is_corrected_then_reconciliation_never_resubmits() -> TestResult {
    for (index, (outcome, audit_mode)) in [
        (ReconciliationOutcome::Reconfirmed, ObservationMode::Missing),
        (ReconciliationOutcome::Failed, ObservationMode::Processed),
        (
            ReconciliationOutcome::Expired,
            ObservationMode::ConfirmedWithoutPostconditions,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let harness = confirmed_harness("audit-correction", index as u128 + 100).await?;
        harness.clock.advance_by(TimeDelta::hours(1))?;
        harness.probe.set_mode(audit_mode)?;
        let audit_time = harness.clock.now();
        let leases = harness.store.claim_confirmation_audit_candidates(
            "rollback-auditor",
            audit_time,
            audit_time + TimeDelta::minutes(1),
            audit_time,
            1,
        )?;
        let submissions_before = harness.probe.submissions.load(Ordering::SeqCst);
        let (_stop_tx, stop_rx) = watch::channel(false);
        let summary = Arc::clone(&harness.fleet)
            .audit_claimed(leases, stop_rx)
            .await;
        assert_eq!(summary.orphaned, 1);
        assert!(summary.errors.is_empty());
        assert_orphan_correction(&harness)?;

        match outcome {
            ReconciliationOutcome::Reconfirmed => {
                harness.probe.set_mode(ObservationMode::Confirmed)?;
            }
            ReconciliationOutcome::Failed => {
                harness.probe.set_mode(ObservationMode::Failed)?;
            }
            ReconciliationOutcome::Expired => {
                harness.probe.set_mode(ObservationMode::Missing)?;
                harness.probe.block_height.store(101, Ordering::SeqCst);
            }
        }
        let leases = harness.store.claim_reconciliation_candidates(
            "orphan-reconciler",
            audit_time,
            audit_time + TimeDelta::minutes(1),
            1,
        )?;
        assert_eq!(leases.len(), 1);
        let (_stop_tx, stop_rx) = watch::channel(false);
        let summary = Arc::clone(&harness.fleet)
            .reconcile_claimed(leases, stop_rx)
            .await;
        let expected_state = match outcome {
            ReconciliationOutcome::Reconfirmed => {
                assert_eq!(summary.confirmed, 1);
                ActionState::Confirmed
            }
            ReconciliationOutcome::Failed => {
                assert_eq!(summary.failed, 1);
                ActionState::Failed
            }
            ReconciliationOutcome::Expired => {
                assert_eq!(summary.expired, 1);
                ActionState::Expired
            }
        };
        assert!(summary.errors.is_empty());
        assert_eq!(
            harness.store.get_action_state(&harness.action.id)?,
            expected_state
        );
        assert_eq!(
            harness.probe.submissions.load(Ordering::SeqCst),
            submissions_before
        );
        let transitions: Vec<_> = harness
            .store
            .action_events(&harness.action.id)?
            .into_iter()
            .filter_map(|event| event.from_state.zip(event.to_state))
            .collect();
        assert!(transitions.contains(&(ActionState::Orphaned, ActionState::Unknown)));
        assert!(transitions.contains(&(ActionState::Unknown, expected_state)));
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum ReconciliationOutcome {
    Reconfirmed,
    Failed,
    Expired,
}

fn assert_orphan_correction(harness: &Harness) -> TestResult {
    assert_eq!(
        harness.store.get_action_state(&harness.action.id)?,
        ActionState::Orphaned
    );
    assert_eq!(
        harness.store.confirmation_audits(&harness.action.id)?[0].outcome,
        ConfirmationAuditOutcome::Orphaned
    );
    let traces = harness.store.traces_for_run(harness.action.run_id)?;
    assert_eq!(
        traces
            .last()
            .and_then(|trace| trace.attributes.get("correction"))
            .map(String::as_str),
        Some("confirmation_orphaned")
    );
    Ok(())
}

async fn confirmed_harness(surfnet: &str, id_offset: u128) -> Result<Harness, Box<dyn Error>> {
    let directory = TempDir::new()?;
    let now = fixed_time()?;
    let clock = Arc::new(VirtualClock::new(now));
    let store = Arc::new(Store::open(
        directory.path().join("audit.sqlite"),
        StoreIdentity::surfpool(format!("{surfnet}-{id_offset}"))?,
    )?);
    let probe = Arc::new(Probe::new());
    let run_id = RunId(Uuid::from_u128(id_offset + 1));
    let agent_a = AgentId(Uuid::from_u128(id_offset + 2));
    let agent_b = AgentId(Uuid::from_u128(id_offset + 3));
    let action = action(run_id, agent_b, now);
    assert!(store.enqueue_action(&action)?);
    let engines = BTreeMap::from([
        (
            agent_a,
            engine(
                Arc::clone(&store),
                Arc::clone(&probe),
                Arc::clone(&clock),
                SIGNER_A,
            )?,
        ),
        (
            agent_b,
            engine(
                Arc::clone(&store),
                Arc::clone(&probe),
                Arc::clone(&clock),
                SIGNER_B,
            )?,
        ),
    ]);
    let fleet = Arc::new(FleetRuntime::new(
        Arc::clone(&store) as Arc<dyn StateStore>,
        engines,
        2,
        Arc::clone(&clock) as Arc<dyn Clock>,
    )?);
    let leases = store.claim_due_actions("initial-worker", now, now + TimeDelta::minutes(1), 1)?;
    let result = fleet
        .execute(leases.first().ok_or("missing initial lease")?)
        .await?;
    assert!(matches!(
        result,
        cooker_runtime::ExecutionResult::Confirmed { .. }
    ));
    assert_eq!(probe.submissions.load(Ordering::SeqCst), 1);
    Ok(Harness {
        _directory: directory,
        store,
        clock,
        probe,
        fleet,
        action,
    })
}

fn engine(
    store: Arc<Store>,
    probe: Arc<Probe>,
    clock: Arc<VirtualClock>,
    signer: &str,
) -> Result<Arc<RuntimeEngine>, CookerError> {
    let gateway: Arc<dyn ChainGateway> = Arc::new(ProbeGateway {
        probe: Arc::clone(&probe),
    });
    let adapter: Arc<dyn ActionAdapter> = Arc::new(ProbeAdapter {
        probe,
        clock: Arc::clone(&clock),
    });
    RuntimeEngine::new(
        store,
        gateway,
        Arc::new(AllowAll),
        [(ActionKind::NativeTransfer, adapter)],
        RuntimeSettings {
            adapter_context: AdapterContext {
                rpc_url: Url::parse("http://127.0.0.1:8899")
                    .map_err(|error| CookerError::InvalidConfig(error.to_string()))?,
                signer: signer.to_owned(),
                confirmation_timeout: Duration::from_secs(5),
            },
            max_concurrency: 2,
        },
        Arc::new(NoFaults),
        clock,
    )
    .map(Arc::new)
}

fn action(run_id: RunId, agent_id: AgentId, now: DateTime<Utc>) -> PlannedAction {
    PlannedAction {
        id: ActionId::derive(run_id, agent_id, 0, "confirmation-audit-test-v1"),
        run_id,
        agent_id,
        sequence: 0,
        model_version: "confirmation-audit-test-v1".to_owned(),
        scheduled_at: now,
        payload: ActionPayload::NativeTransfer {
            destination: "audit-destination".to_owned(),
            lamports: 50_000,
        },
        max_fee_lamports: 5_000,
        max_account_creation_lamports: 0,
        created_at: now,
    }
}

fn fixed_time() -> Result<DateTime<Utc>, Box<dyn Error>> {
    Utc.with_ymd_and_hms(2026, 7, 17, 12, 0, 0)
        .single()
        .ok_or_else(|| "invalid fixed time".into())
}
