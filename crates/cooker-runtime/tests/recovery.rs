//! Durable runtime lifecycle and six-checkpoint restart proof.

use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use chrono::{DateTime, TimeDelta, TimeZone, Utc};
use cooker_core::{
    ActionAdapter, ActionId, ActionKind, ActionPayload, ActionState, AdapterContext, AgentId,
    BudgetConfig, ChainGateway, ChainReceipt, Clock, ConfirmationStatus, CookerError,
    PlannedAction, Policy, PolicyConfig, PolicyDecision, PreparedAction, RunId, SafetyPolicy,
    SimulationReceipt, StateExpectation, StateStore, VirtualClock,
};
use cooker_runtime::{
    ExecutionCheckpoint, ExecutionResult, FaultInjector, NoFaults, RuntimeEngine, RuntimeSettings,
};
use cooker_store::{Store, StoreIdentity};
use tempfile::TempDir;
use url::Url;
use uuid::Uuid;

type TestResult = Result<(), Box<dyn Error>>;

#[derive(Debug, Default)]
struct ChainState {
    landed: AtomicBool,
    preparations: AtomicUsize,
    submissions: AtomicUsize,
    block_height: AtomicU64,
}

#[derive(Debug)]
struct FakeGateway {
    state: Arc<ChainState>,
    clock: Arc<VirtualClock>,
}

#[async_trait]
impl ChainGateway for FakeGateway {
    async fn simulate(&self, _transaction: &[u8]) -> Result<SimulationReceipt, CookerError> {
        Ok(SimulationReceipt {
            succeeded: true,
            units_consumed: Some(400),
            logs: vec!["local simulation".to_owned()],
            error: None,
        })
    }

    async fn submit(&self, _transaction: &[u8]) -> Result<String, CookerError> {
        self.state.submissions.fetch_add(1, Ordering::SeqCst);
        self.state.landed.store(true, Ordering::SeqCst);
        Ok("local-signature".to_owned())
    }

    async fn observe(
        &self,
        action_id: &ActionId,
        signature: &str,
    ) -> Result<ChainReceipt, CookerError> {
        Ok(receipt(
            action_id.clone(),
            signature,
            self.state.landed.load(Ordering::SeqCst),
            self.clock.now(),
        ))
    }

    async fn native_balance(&self, _address: &str) -> Result<u64, CookerError> {
        Ok(10_000_000_000)
    }

    async fn block_height(&self) -> Result<u64, CookerError> {
        Ok(self.state.block_height.load(Ordering::SeqCst))
    }
}

#[derive(Debug)]
struct FakeAdapter {
    state: Arc<ChainState>,
    clock: Arc<VirtualClock>,
}

#[async_trait]
impl ActionAdapter for FakeAdapter {
    fn supports(&self, payload: &ActionPayload) -> bool {
        matches!(payload, ActionPayload::NativeTransfer { .. })
    }

    async fn prepare(
        &self,
        _context: &AdapterContext,
        action: &PlannedAction,
    ) -> Result<PreparedAction, CookerError> {
        self.state.preparations.fetch_add(1, Ordering::SeqCst);
        Ok(PreparedAction {
            action_id: action.id.clone(),
            signature: "local-signature".to_owned(),
            transaction: vec![1, 2, 3, 4],
            recent_blockhash: "local-blockhash".to_owned(),
            last_valid_block_height: 100,
            expectations: vec![StateExpectation {
                kind: "native_balance_delta".to_owned(),
                account: "destination".to_owned(),
                expected_delta: Some(50_000),
                attributes: BTreeMap::new(),
            }],
        })
    }

    async fn observe(
        &self,
        _context: &AdapterContext,
        action: &PlannedAction,
        prepared: &PreparedAction,
    ) -> Result<ChainReceipt, CookerError> {
        Ok(receipt(
            action.id.clone(),
            &prepared.signature,
            self.state.landed.load(Ordering::SeqCst),
            self.clock.now(),
        ))
    }
}

#[derive(Debug)]
struct AlwaysDelay {
    until: DateTime<Utc>,
}

#[derive(Debug)]
struct RewriteOnce;

impl Policy for RewriteOnce {
    fn evaluate(
        &self,
        action: &PlannedAction,
        _signer_balance: u64,
        _spent_today: u64,
        _spent_lifetime: u64,
    ) -> Result<PolicyDecision, CookerError> {
        match &action.payload {
            ActionPayload::NativeTransfer { destination, .. } if destination == "destination" => {
                Ok(PolicyDecision::Rewrite {
                    payload: ActionPayload::NativeTransfer {
                        destination: "rewritten-destination".to_owned(),
                        lamports: 25_000,
                    },
                    reason: "bounded_rewrite".to_owned(),
                })
            }
            ActionPayload::NativeTransfer { destination, .. }
                if destination == "rewritten-destination" =>
            {
                Ok(PolicyDecision::Allow)
            }
            _ => Ok(PolicyDecision::Reject {
                reason: "unexpected_payload".to_owned(),
            }),
        }
    }
}

impl Policy for AlwaysDelay {
    fn evaluate(
        &self,
        _action: &PlannedAction,
        _signer_balance: u64,
        _spent_today: u64,
        _spent_lifetime: u64,
    ) -> Result<PolicyDecision, CookerError> {
        Ok(PolicyDecision::Delay {
            until: self.until,
            reason: "test_delay".to_owned(),
        })
    }
}

#[derive(Clone, Copy, Debug)]
struct CrashAt(ExecutionCheckpoint);

impl FaultInjector for CrashAt {
    fn check(&self, checkpoint: ExecutionCheckpoint) -> Result<(), CookerError> {
        if self.0 == checkpoint {
            Err(CookerError::Store(format!(
                "injected crash at {checkpoint:?}"
            )))
        } else {
            Ok(())
        }
    }
}

fn receipt(
    action_id: ActionId,
    signature: &str,
    landed: bool,
    observed_at: DateTime<Utc>,
) -> ChainReceipt {
    ChainReceipt {
        action_id,
        signature: signature.to_owned(),
        status: if landed {
            ConfirmationStatus::Confirmed
        } else {
            ConfirmationStatus::Missing
        },
        slot: landed.then_some(42),
        observed_at,
        postconditions_met: landed,
        observations: landed
            .then(|| StateExpectation {
                kind: "native_balance_delta".to_owned(),
                account: "destination".to_owned(),
                expected_delta: Some(50_000),
                attributes: BTreeMap::new(),
            })
            .into_iter()
            .collect(),
        error: None,
    }
}

fn action(now: DateTime<Utc>) -> PlannedAction {
    let run_id = RunId(Uuid::from_u128(1));
    let agent_id = AgentId(Uuid::from_u128(2));
    PlannedAction {
        id: ActionId::derive(run_id, agent_id, 0, "runtime-test-v1"),
        run_id,
        agent_id,
        sequence: 0,
        model_version: "runtime-test-v1".to_owned(),
        scheduled_at: now,
        payload: ActionPayload::NativeTransfer {
            destination: "destination".to_owned(),
            lamports: 50_000,
        },
        max_fee_lamports: 5_000,
        max_account_creation_lamports: 0,
        created_at: now,
    }
}

fn policy() -> Result<SafetyPolicy, CookerError> {
    SafetyPolicy::new(PolicyConfig {
        budgets: BudgetConfig {
            daily_lamports: 1_000_000,
            lifetime_lamports: 10_000_000,
            reserve_lamports: 100_000,
            max_fee_lamports: 10_000,
            max_account_creation_lamports: 10_000_000,
        },
        allowed_actions: BTreeSet::from([ActionKind::NativeTransfer]),
        max_action_amounts: BTreeMap::from([(ActionKind::NativeTransfer, 100_000)]),
        allowed_destinations: BTreeSet::from(["destination".to_owned()]),
        allowed_mints: BTreeSet::new(),
        native_input_mints: BTreeSet::new(),
        max_slippage_bps: 100,
    })
}

fn runtime(
    store: Arc<Store>,
    state: Arc<ChainState>,
    clock: Arc<VirtualClock>,
    faults: Arc<dyn FaultInjector>,
) -> Result<RuntimeEngine, CookerError> {
    runtime_with_policy(store, state, clock, faults, Arc::new(policy()?))
}

fn runtime_with_policy(
    store: Arc<Store>,
    state: Arc<ChainState>,
    clock: Arc<VirtualClock>,
    faults: Arc<dyn FaultInjector>,
    policy: Arc<dyn Policy>,
) -> Result<RuntimeEngine, CookerError> {
    let gateway: Arc<dyn ChainGateway> = Arc::new(FakeGateway {
        state: Arc::clone(&state),
        clock: Arc::clone(&clock),
    });
    let adapter: Arc<dyn ActionAdapter> = Arc::new(FakeAdapter {
        state,
        clock: Arc::clone(&clock),
    });
    RuntimeEngine::new(
        store,
        gateway,
        policy,
        [(ActionKind::NativeTransfer, adapter)],
        RuntimeSettings {
            adapter_context: AdapterContext {
                rpc_url: Url::parse("http://127.0.0.1:8899")
                    .map_err(|error| CookerError::InvalidConfig(error.to_string()))?,
                signer: "local-signer".to_owned(),
                confirmation_timeout: Duration::from_secs(5),
            },
            max_concurrency: 4,
        },
        faults,
        clock,
    )
}

fn fixed_time() -> Result<DateTime<Utc>, Box<dyn Error>> {
    Utc.with_ymd_and_hms(2026, 7, 16, 12, 0, 0)
        .single()
        .ok_or_else(|| "invalid fixed time".into())
}

#[tokio::test]
async fn successful_lifecycle_confirms_once() -> TestResult {
    let directory = TempDir::new()?;
    let path = directory.path().join("runtime.sqlite");
    let now = fixed_time()?;
    let clock = Arc::new(VirtualClock::new(now));
    let store = Arc::new(Store::open(
        &path,
        StoreIdentity::surfpool("runtime-test")?,
    )?);
    let planned = action(now);
    assert!(store.enqueue_action(&planned)?);
    let leases = store.claim_due_actions("worker", now, now + TimeDelta::minutes(1), 1)?;
    let lease = leases.first().ok_or("missing lease")?;
    let state = Arc::new(ChainState::default());
    let runtime = runtime(store.clone(), state.clone(), clock, Arc::new(NoFaults))?;

    let result = runtime.execute(lease).await?;
    assert!(matches!(result, ExecutionResult::Confirmed { .. }));
    assert_eq!(store.get_action_state(&planned.id)?, ActionState::Confirmed);
    assert_eq!(state.submissions.load(Ordering::SeqCst), 1);
    let traces = store.traces_for_run(planned.run_id)?;
    assert_eq!(traces.len(), 1);
    assert_eq!(traces[0].action_id, planned.id);
    assert_eq!(traces[0].signature.as_deref(), Some("local-signature"));
    assert_eq!(
        traces[0].attributes.get("status").map(String::as_str),
        Some("confirmed")
    );
    assert_eq!(
        traces[0]
            .attributes
            .get("observation.0.delta")
            .map(String::as_str),
        Some("50000")
    );
    Ok(())
}

#[tokio::test]
async fn policy_delay_is_persisted_before_the_lease_is_released() -> TestResult {
    let directory = TempDir::new()?;
    let path = directory.path().join("delay.sqlite");
    let now = fixed_time()?;
    let until = now + TimeDelta::days(1);
    let clock = Arc::new(VirtualClock::new(now));
    let store = Arc::new(Store::open(
        &path,
        StoreIdentity::surfpool("runtime-delay-test")?,
    )?);
    let planned = action(now);
    assert!(store.enqueue_action(&planned)?);
    let leases = store.claim_due_actions("worker", now, now + TimeDelta::days(2), 1)?;
    let chain = Arc::new(ChainState::default());
    let runtime = runtime_with_policy(
        Arc::clone(&store),
        Arc::clone(&chain),
        Arc::clone(&clock),
        Arc::new(NoFaults),
        Arc::new(AlwaysDelay { until }),
    )?;

    let result = runtime
        .execute(leases.first().ok_or("missing delay lease")?)
        .await?;
    assert!(matches!(result, ExecutionResult::Delayed { .. }));
    assert_eq!(store.get_action_state(&planned.id)?, ActionState::Planned);
    assert_eq!(chain.submissions.load(Ordering::SeqCst), 0);
    assert!(
        store
            .claim_due_actions(
                "early-worker",
                until - TimeDelta::seconds(1),
                until + TimeDelta::minutes(1),
                1,
            )?
            .is_empty()
    );
    assert_eq!(
        store
            .claim_due_actions("due-worker", until, until + TimeDelta::minutes(1), 1)?
            .len(),
        1
    );
    Ok(())
}

#[tokio::test]
async fn policy_rewrite_is_persisted_rechecked_and_executed_once() -> TestResult {
    let directory = TempDir::new()?;
    let path = directory.path().join("rewrite.sqlite");
    let now = fixed_time()?;
    let clock = Arc::new(VirtualClock::new(now));
    let store = Arc::new(Store::open(
        &path,
        StoreIdentity::surfpool("runtime-rewrite-test")?,
    )?);
    let original = action(now);
    assert!(store.enqueue_action(&original)?);
    let leases = store.claim_due_actions("worker", now, now + TimeDelta::minutes(1), 1)?;
    let chain = Arc::new(ChainState::default());
    let runtime = runtime_with_policy(
        Arc::clone(&store),
        Arc::clone(&chain),
        Arc::clone(&clock),
        Arc::new(NoFaults),
        Arc::new(RewriteOnce),
    )?;

    let result = runtime
        .execute(leases.first().ok_or("missing rewrite lease")?)
        .await?;
    assert!(matches!(result, ExecutionResult::Confirmed { .. }));
    assert_eq!(chain.preparations.load(Ordering::SeqCst), 1);
    assert_eq!(chain.submissions.load(Ordering::SeqCst), 1);
    assert!(matches!(
        store.get_action(&original.id)?.payload,
        ActionPayload::NativeTransfer {
            ref destination,
            lamports: 25_000,
        } if destination == "rewritten-destination"
    ));
    assert!(
        store
            .action_events(&original.id)?
            .iter()
            .any(|event| event.kind == "policy_rewritten")
    );
    Ok(())
}

#[tokio::test]
async fn every_crash_checkpoint_recovers_without_duplicate_submission() -> TestResult {
    for checkpoint in [
        ExecutionCheckpoint::AfterIntentPersistence,
        ExecutionCheckpoint::AfterPreparedPersistence,
        ExecutionCheckpoint::AfterSimulation,
        ExecutionCheckpoint::AfterSignaturePersistence,
        ExecutionCheckpoint::AfterSendResponseLost,
        ExecutionCheckpoint::AfterConfirmationBeforePromotion,
    ] {
        run_recovery_case(checkpoint).await?;
    }
    Ok(())
}

async fn run_recovery_case(checkpoint: ExecutionCheckpoint) -> TestResult {
    let directory = TempDir::new()?;
    let path = directory.path().join("recovery.sqlite");
    let now = fixed_time()?;
    let clock = Arc::new(VirtualClock::new(now));
    let chain = Arc::new(ChainState::default());
    let planned = action(now);

    {
        let store = Arc::new(Store::open(
            &path,
            StoreIdentity::surfpool("recovery-test")?,
        )?);
        store.enqueue_action(&planned)?;
        let leases = store.claim_due_actions("first", now, now + TimeDelta::minutes(1), 1)?;
        let lease = leases.first().ok_or("missing first lease")?;
        let runtime = runtime(
            store,
            Arc::clone(&chain),
            Arc::clone(&clock),
            Arc::new(CrashAt(checkpoint)),
        )?;
        let _result = runtime.execute(lease).await;
    }

    clock.advance_by(TimeDelta::minutes(2))?;
    let recovery_time = clock.now();
    chain.block_height.store(200, Ordering::SeqCst);
    let store = Arc::new(Store::open(
        &path,
        StoreIdentity::surfpool("recovery-test")?,
    )?);
    let runtime = runtime(
        store.clone(),
        Arc::clone(&chain),
        Arc::clone(&clock),
        Arc::new(NoFaults),
    )?;
    let state_after_crash = store.get_action_state(&planned.id)?;
    let result = match state_after_crash {
        ActionState::Planned | ActionState::Simulated => {
            let leases = store.claim_due_actions(
                "restart-normal",
                recovery_time,
                recovery_time + TimeDelta::minutes(1),
                1,
            )?;
            runtime
                .execute(leases.first().ok_or("missing restart lease")?)
                .await?
        }
        ActionState::Submitted | ActionState::Unknown => {
            let leases = store.claim_reconciliation_candidates(
                "restart-reconcile",
                recovery_time,
                recovery_time + TimeDelta::minutes(1),
                1,
            )?;
            runtime
                .reconcile(leases.first().ok_or("missing reconciliation lease")?)
                .await?
        }
        state => return Err(format!("unexpected crash state {state:?}").into()),
    };

    let submissions = chain.submissions.load(Ordering::SeqCst);
    assert_eq!(
        chain.preparations.load(Ordering::SeqCst),
        1,
        "checkpoint {checkpoint:?} prepared more than one transaction"
    );
    if checkpoint == ExecutionCheckpoint::AfterSignaturePersistence {
        assert!(matches!(result, ExecutionResult::Expired { .. }));
        assert_eq!(submissions, 0);
        assert_eq!(store.get_action_state(&planned.id)?, ActionState::Expired);
    } else {
        assert!(matches!(result, ExecutionResult::Confirmed { .. }));
        assert_eq!(submissions, 1);
        assert_eq!(store.get_action_state(&planned.id)?, ActionState::Confirmed);
    }
    assert_eq!(
        store
            .action_events(&planned.id)?
            .iter()
            .filter(|event| event.to_state == Some(ActionState::Confirmed))
            .count(),
        usize::from(checkpoint != ExecutionCheckpoint::AfterSignaturePersistence)
    );
    let traces = store.traces_for_run(planned.run_id)?;
    assert_eq!(traces.len(), 1, "checkpoint {checkpoint:?}");
    assert_eq!(traces[0].action_id, planned.id);
    Ok(())
}
