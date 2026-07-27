//! Durable multi-signer routing, bounded concurrency, and shutdown proofs.

#![forbid(unsafe_code)]

use std::{
    collections::BTreeMap,
    error::Error,
    str,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use chrono::{DateTime, TimeDelta, TimeZone, Utc};
use cooker_core::{
    ActionAdapter, ActionId, ActionKind, ActionPayload, ActionState, AdapterContext, AgentId,
    ChainGateway, ChainReceipt, Clock, ConfirmationStatus, CookerError, PlannedAction, Policy,
    PolicyDecision, PreparedAction, RunId, SimulationReceipt, StateExpectation, StateStore,
    VirtualClock,
};
use cooker_runtime::{FleetRuntime, NoFaults, RuntimeEngine, RuntimeSettings};
use cooker_store::{Store, StoreIdentity};
use tempfile::TempDir;
use tokio::sync::watch;
use url::Url;
use uuid::Uuid;

type TestResult = Result<(), Box<dyn Error>>;

const SIGNER_A: &str = "fleet-signer-a";
const SIGNER_B: &str = "fleet-signer-b";

#[derive(Debug, Default)]
struct Probe {
    active_prepares: AtomicUsize,
    peak_prepares: AtomicUsize,
    prepare_calls: AtomicUsize,
    submissions: Mutex<Vec<String>>,
    observations: Mutex<Vec<(String, ActionId)>>,
}

impl Probe {
    fn record_submission(&self, signature: String) -> Result<(), CookerError> {
        self.submissions
            .lock()
            .map_err(|_| CookerError::Chain("submission probe lock poisoned".to_owned()))?
            .push(signature);
        Ok(())
    }

    fn record_observation(&self, signer: String, action_id: ActionId) -> Result<(), CookerError> {
        self.observations
            .lock()
            .map_err(|_| CookerError::Chain("observation probe lock poisoned".to_owned()))?
            .push((signer, action_id));
        Ok(())
    }

    fn submissions(&self) -> Result<Vec<String>, CookerError> {
        self.submissions
            .lock()
            .map_err(|_| CookerError::Chain("submission probe lock poisoned".to_owned()))
            .map(|submissions| submissions.clone())
    }

    fn observations(&self) -> Result<Vec<(String, ActionId)>, CookerError> {
        self.observations
            .lock()
            .map_err(|_| CookerError::Chain("observation probe lock poisoned".to_owned()))
            .map(|observations| observations.clone())
    }
}

#[derive(Debug)]
struct ProbeGateway {
    probe: Arc<Probe>,
    clock: Arc<VirtualClock>,
}

#[async_trait]
impl ChainGateway for ProbeGateway {
    async fn simulate(&self, _transaction: &[u8]) -> Result<SimulationReceipt, CookerError> {
        Ok(successful_simulation())
    }

    async fn submit(&self, transaction: &[u8]) -> Result<String, CookerError> {
        let signature = str::from_utf8(transaction)
            .map_err(|error| CookerError::Chain(format!("invalid test transaction: {error}")))?
            .to_owned();
        self.probe.record_submission(signature.clone())?;
        Ok(signature)
    }

    async fn observe(
        &self,
        action_id: &ActionId,
        signature: &str,
    ) -> Result<ChainReceipt, CookerError> {
        Ok(confirmed_receipt(
            action_id.clone(),
            signature,
            self.clock.now(),
        ))
    }

    async fn native_balance(&self, _address: &str) -> Result<u64, CookerError> {
        Ok(10_000_000_000)
    }

    async fn block_height(&self) -> Result<u64, CookerError> {
        Ok(1)
    }
}

#[derive(Debug)]
struct ProbeAdapter {
    probe: Arc<Probe>,
    clock: Arc<VirtualClock>,
    prepare_delay: Duration,
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
        self.probe.prepare_calls.fetch_add(1, Ordering::SeqCst);
        let active = self.probe.active_prepares.fetch_add(1, Ordering::SeqCst) + 1;
        self.probe.peak_prepares.fetch_max(active, Ordering::SeqCst);
        tokio::time::sleep(self.prepare_delay).await;
        self.probe.active_prepares.fetch_sub(1, Ordering::SeqCst);
        Ok(prepared(action, &context.signer))
    }

    async fn observe(
        &self,
        context: &AdapterContext,
        action: &PlannedAction,
        prepared: &PreparedAction,
    ) -> Result<ChainReceipt, CookerError> {
        self.probe
            .record_observation(context.signer.clone(), action.id.clone())?;
        Ok(confirmed_receipt(
            action.id.clone(),
            &prepared.signature,
            self.clock.now(),
        ))
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

#[tokio::test]
async fn durable_agents_route_to_distinct_signer_engines_and_signatures() -> TestResult {
    let directory = TempDir::new()?;
    let now = fixed_time()?;
    let clock = Arc::new(VirtualClock::new(now));
    let store = open_store(&directory, "fleet-routing")?;
    let probe = Arc::new(Probe::default());
    let run_id = RunId(Uuid::from_u128(10));
    let agent_a = AgentId(Uuid::from_u128(11));
    let agent_b = AgentId(Uuid::from_u128(12));
    let action_a = action(run_id, agent_a, 0, now);
    let action_b = action(run_id, agent_b, 0, now);
    assert!(store.enqueue_action(&action_a)?);
    assert!(store.enqueue_action(&action_b)?);
    let leases = store.claim_due_actions("fleet", now, now + TimeDelta::minutes(1), 2)?;
    assert_eq!(leases.len(), 2);

    let fleet = fleet(
        Arc::clone(&store),
        &probe,
        Arc::clone(&clock),
        [(agent_a, SIGNER_A), (agent_b, SIGNER_B)],
        2,
        Duration::ZERO,
    )?;
    let (_stop_tx, stop_rx) = watch::channel(false);
    let summary = fleet.run_claimed(leases, stop_rx).await;

    assert_eq!(summary.confirmed, 2);
    assert!(summary.errors.is_empty());
    let mut submitted = probe.submissions()?;
    submitted.sort();
    let mut expected = vec![
        signature(&action_a, SIGNER_A),
        signature(&action_b, SIGNER_B),
    ];
    expected.sort();
    assert_eq!(submitted, expected);
    assert_eq!(
        store.get_submission_signature(&action_a.id)?.as_deref(),
        Some(signature(&action_a, SIGNER_A).as_str())
    );
    assert_eq!(
        store.get_submission_signature(&action_b.id)?.as_deref(),
        Some(signature(&action_b, SIGNER_B).as_str())
    );
    let signer_by_action: BTreeMap<_, _> = store
        .traces_for_run(run_id)?
        .into_iter()
        .map(|trace| {
            let signer = trace.attributes.get("signer").cloned();
            (trace.action_id, signer)
        })
        .collect();
    assert_eq!(
        signer_by_action
            .get(&action_a.id)
            .and_then(Option::as_deref),
        Some(SIGNER_A)
    );
    assert_eq!(
        signer_by_action
            .get(&action_b.id)
            .and_then(Option::as_deref),
        Some(SIGNER_B)
    );
    Ok(())
}

#[test]
fn duplicate_signer_registration_is_rejected() -> TestResult {
    let directory = TempDir::new()?;
    let now = fixed_time()?;
    let clock = Arc::new(VirtualClock::new(now));
    let store = open_store(&directory, "fleet-duplicate-signer")?;
    let probe = Arc::new(Probe::default());
    let agent_a = AgentId(Uuid::from_u128(21));
    let agent_b = AgentId(Uuid::from_u128(22));
    let engines = BTreeMap::from([
        (
            agent_a,
            engine(
                Arc::clone(&store),
                Arc::clone(&probe),
                Arc::clone(&clock),
                SIGNER_A,
                Duration::ZERO,
            )?,
        ),
        (
            agent_b,
            engine(
                Arc::clone(&store),
                probe,
                Arc::clone(&clock),
                SIGNER_A,
                Duration::ZERO,
            )?,
        ),
    ]);

    let result = FleetRuntime::new(store, engines, 2, clock);
    assert!(matches!(
        result,
        Err(CookerError::InvalidConfig(message))
            if message.contains("assigned to more than one agent")
    ));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn delayed_adapters_never_exceed_fleet_concurrency() -> TestResult {
    let directory = TempDir::new()?;
    let now = fixed_time()?;
    let clock = Arc::new(VirtualClock::new(now));
    let store = open_store(&directory, "fleet-concurrency")?;
    let probe = Arc::new(Probe::default());
    let run_id = RunId(Uuid::from_u128(30));
    let agent_a = AgentId(Uuid::from_u128(31));
    let agent_b = AgentId(Uuid::from_u128(32));
    for sequence in 0..8 {
        let agent = if sequence % 2 == 0 { agent_a } else { agent_b };
        assert!(store.enqueue_action(&action(run_id, agent, sequence, now))?);
    }
    let leases = store.claim_due_actions("fleet", now, now + TimeDelta::minutes(1), 8)?;
    assert_eq!(leases.len(), 8);
    let fleet = fleet(
        Arc::clone(&store),
        &probe,
        Arc::clone(&clock),
        [(agent_a, SIGNER_A), (agent_b, SIGNER_B)],
        2,
        Duration::from_millis(30),
    )?;
    let (_stop_tx, stop_rx) = watch::channel(false);

    let summary = fleet.run_claimed(leases, stop_rx).await;

    assert_eq!(summary.confirmed, 8);
    assert!(summary.errors.is_empty());
    assert_eq!(summary.peak_workers, 2);
    assert_eq!(probe.peak_prepares.load(Ordering::SeqCst), 2);
    assert_eq!(probe.active_prepares.load(Ordering::SeqCst), 0);
    assert_eq!(probe.prepare_calls.load(Ordering::SeqCst), 8);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_agent_never_executes_two_actions_concurrently() -> TestResult {
    let directory = TempDir::new()?;
    let now = fixed_time()?;
    let clock = Arc::new(VirtualClock::new(now));
    let store = open_store(&directory, "fleet-agent-serialization")?;
    let probe = Arc::new(Probe::default());
    let run_id = RunId(Uuid::from_u128(35));
    let agent = AgentId(Uuid::from_u128(36));
    for sequence in 0..8 {
        assert!(store.enqueue_action(&action(run_id, agent, sequence, now))?);
    }
    let leases = store.claim_due_actions("fleet", now, now + TimeDelta::minutes(1), 8)?;
    let fleet = fleet(
        Arc::clone(&store),
        &probe,
        Arc::clone(&clock),
        [(agent, SIGNER_A)],
        4,
        Duration::from_millis(20),
    )?;
    let (_stop_tx, stop_rx) = watch::channel(false);

    let summary = fleet.run_claimed(leases, stop_rx).await;

    assert_eq!(summary.confirmed, 8);
    assert!(summary.errors.is_empty());
    assert_eq!(summary.peak_workers, 4);
    assert_eq!(probe.peak_prepares.load(Ordering::SeqCst), 1);
    assert_eq!(probe.active_prepares.load(Ordering::SeqCst), 0);
    Ok(())
}

#[tokio::test]
async fn preset_kill_switch_releases_all_unstarted_leases_for_reclaim() -> TestResult {
    let directory = TempDir::new()?;
    let now = fixed_time()?;
    let clock = Arc::new(VirtualClock::new(now));
    let store = open_store(&directory, "fleet-kill-switch")?;
    let probe = Arc::new(Probe::default());
    let run_id = RunId(Uuid::from_u128(40));
    let agent_a = AgentId(Uuid::from_u128(41));
    let agent_b = AgentId(Uuid::from_u128(42));
    let mut actions = Vec::new();
    for sequence in 0..5 {
        let agent = if sequence % 2 == 0 { agent_a } else { agent_b };
        let planned = action(run_id, agent, sequence, now);
        assert!(store.enqueue_action(&planned)?);
        actions.push(planned);
    }
    let leases = store.claim_due_actions("stopped", now, now + TimeDelta::minutes(1), 5)?;
    let fleet = fleet(
        Arc::clone(&store),
        &probe,
        Arc::clone(&clock),
        [(agent_a, SIGNER_A), (agent_b, SIGNER_B)],
        2,
        Duration::from_secs(1),
    )?;
    let (_stop_tx, stop_rx) = watch::channel(true);

    let summary = fleet.run_claimed(leases, stop_rx).await;

    assert_eq!(summary.released_without_start, 5);
    assert_eq!(summary.peak_workers, 0);
    assert!(summary.errors.is_empty());
    assert_eq!(probe.prepare_calls.load(Ordering::SeqCst), 0);
    assert!(probe.submissions()?.is_empty());
    for planned in &actions {
        assert_eq!(store.get_action_state(&planned.id)?, ActionState::Planned);
    }
    let reclaimed =
        store.claim_due_actions("restarted", now, now + TimeDelta::minutes(1), actions.len())?;
    assert_eq!(reclaimed.len(), actions.len());
    Ok(())
}

#[tokio::test]
async fn reconciliation_uses_original_signer_and_never_submits() -> TestResult {
    let directory = TempDir::new()?;
    let now = fixed_time()?;
    let clock = Arc::new(VirtualClock::new(now));
    let store = open_store(&directory, "fleet-reconciliation")?;
    let probe = Arc::new(Probe::default());
    let run_id = RunId(Uuid::from_u128(50));
    let agent_a = AgentId(Uuid::from_u128(51));
    let agent_b = AgentId(Uuid::from_u128(52));
    let planned = action(run_id, agent_b, 0, now);
    assert!(store.enqueue_action(&planned)?);
    let setup_leases = store.claim_due_actions("setup", now, now + TimeDelta::minutes(1), 1)?;
    let setup_lease = setup_leases.first().ok_or("missing setup lease")?;
    let signed = prepared(&planned, SIGNER_B);
    store.record_prepared(setup_lease, &signed, now)?;
    store.record_simulation(setup_lease, &successful_simulation(), now)?;
    store.transition_action(
        setup_lease,
        ActionState::Planned,
        ActionState::Simulated,
        now,
        None,
    )?;
    store.record_submission(setup_lease, &signed.signature, now)?;
    store.transition_action(
        setup_lease,
        ActionState::Simulated,
        ActionState::Submitted,
        now,
        None,
    )?;
    store.release_lease(setup_lease, now)?;
    let leases =
        store.claim_reconciliation_candidates("reconcile", now, now + TimeDelta::minutes(1), 1)?;
    let fleet = fleet(
        Arc::clone(&store),
        &probe,
        Arc::clone(&clock),
        [(agent_a, SIGNER_A), (agent_b, SIGNER_B)],
        2,
        Duration::ZERO,
    )?;
    let (_stop_tx, stop_rx) = watch::channel(false);

    let summary = fleet.reconcile_claimed(leases, stop_rx).await;

    assert_eq!(summary.confirmed, 1);
    assert!(summary.errors.is_empty());
    assert!(probe.submissions()?.is_empty());
    assert_eq!(
        probe.observations()?,
        vec![(SIGNER_B.to_owned(), planned.id.clone())]
    );
    assert_eq!(store.get_action_state(&planned.id)?, ActionState::Confirmed);
    assert_eq!(
        store.get_submission_signature(&planned.id)?.as_deref(),
        Some(signed.signature.as_str())
    );
    Ok(())
}

fn fleet<const N: usize>(
    store: Arc<Store>,
    probe: &Arc<Probe>,
    clock: Arc<VirtualClock>,
    routes: [(AgentId, &'static str); N],
    max_concurrency: usize,
    prepare_delay: Duration,
) -> Result<Arc<FleetRuntime>, CookerError> {
    let engines = routes
        .into_iter()
        .map(|(agent_id, signer)| {
            engine(
                Arc::clone(&store),
                Arc::clone(probe),
                Arc::clone(&clock),
                signer,
                prepare_delay,
            )
            .map(|runtime| (agent_id, runtime))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    FleetRuntime::new(store, engines, max_concurrency, clock).map(Arc::new)
}

fn engine(
    store: Arc<Store>,
    probe: Arc<Probe>,
    clock: Arc<VirtualClock>,
    signer: &str,
    prepare_delay: Duration,
) -> Result<Arc<RuntimeEngine>, CookerError> {
    let gateway: Arc<dyn ChainGateway> = Arc::new(ProbeGateway {
        probe: Arc::clone(&probe),
        clock: Arc::clone(&clock),
    });
    let adapter: Arc<dyn ActionAdapter> = Arc::new(ProbeAdapter {
        probe,
        clock: Arc::clone(&clock),
        prepare_delay,
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
            max_concurrency: 16,
        },
        Arc::new(NoFaults),
        clock,
    )
    .map(Arc::new)
}

fn open_store(directory: &TempDir, surfnet_id: &str) -> Result<Arc<Store>, CookerError> {
    Store::open(
        directory.path().join("fleet.sqlite"),
        StoreIdentity::surfpool(surfnet_id)?,
    )
    .map(Arc::new)
}

fn action(run_id: RunId, agent_id: AgentId, sequence: u64, now: DateTime<Utc>) -> PlannedAction {
    PlannedAction {
        id: ActionId::derive(run_id, agent_id, sequence, "fleet-test-v1"),
        run_id,
        agent_id,
        sequence,
        model_version: "fleet-test-v1".to_owned(),
        scheduled_at: now,
        payload: ActionPayload::NativeTransfer {
            destination: format!("destination-{sequence}"),
            lamports: 50_000,
        },
        max_fee_lamports: 5_000,
        max_account_creation_lamports: 0,
        created_at: now,
    }
}

fn signature(action: &PlannedAction, signer: &str) -> String {
    format!("signature:{signer}:{}", action.id)
}

fn prepared(action: &PlannedAction, signer: &str) -> PreparedAction {
    let signature = signature(action, signer);
    PreparedAction {
        action_id: action.id.clone(),
        transaction: signature.as_bytes().to_vec(),
        signature,
        recent_blockhash: "fleet-blockhash".to_owned(),
        last_valid_block_height: 100,
        expectations: vec![StateExpectation {
            kind: "native_balance_delta".to_owned(),
            account: "destination".to_owned(),
            expected_delta: Some(50_000),
            attributes: BTreeMap::new(),
        }],
    }
}

fn successful_simulation() -> SimulationReceipt {
    SimulationReceipt {
        succeeded: true,
        units_consumed: Some(400),
        logs: vec!["local simulation".to_owned()],
        error: None,
    }
}

fn confirmed_receipt(
    action_id: ActionId,
    signature: &str,
    observed_at: DateTime<Utc>,
) -> ChainReceipt {
    ChainReceipt {
        action_id,
        signature: signature.to_owned(),
        status: ConfirmationStatus::Confirmed,
        slot: Some(42),
        observed_at,
        postconditions_met: true,
        observations: vec![StateExpectation {
            kind: "native_balance_delta".to_owned(),
            account: "destination".to_owned(),
            expected_delta: Some(50_000),
            attributes: BTreeMap::new(),
        }],
        error: None,
    }
}

fn fixed_time() -> Result<DateTime<Utc>, Box<dyn Error>> {
    Utc.with_ymd_and_hms(2026, 7, 17, 12, 0, 0)
        .single()
        .ok_or_else(|| "invalid fixed time".into())
}
