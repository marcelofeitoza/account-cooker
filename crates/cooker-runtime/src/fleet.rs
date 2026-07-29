//! Multi-agent routing over isolated single-signer runtimes.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::Arc,
};

use cooker_core::{ActionLease, AgentId, Clock, CookerError, StateStore};
use tokio::sync::{Mutex, watch};

use crate::{ExecutionResult, RuntimeEngine, WorkerSummary, workers::drain_bounded};

/// Runtime router that binds every durable agent to one unique signer engine.
pub struct FleetRuntime {
    store: Arc<dyn StateStore>,
    engines: BTreeMap<AgentId, Arc<RuntimeEngine>>,
    agent_gates: BTreeMap<AgentId, Arc<Mutex<()>>>,
    clock: Arc<dyn Clock>,
    max_concurrency: usize,
}

impl fmt::Debug for FleetRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FleetRuntime")
            .field("agent_count", &self.engines.len())
            .field("max_concurrency", &self.max_concurrency)
            .finish_non_exhaustive()
    }
}

impl FleetRuntime {
    /// Construct a fleet and reject missing routes, shared signers, or zero concurrency.
    ///
    /// # Errors
    ///
    /// Returns an error when no agents are registered, concurrency is zero, a signer address is
    /// empty, or multiple agent identities point to the same signer.
    pub fn new(
        store: Arc<dyn StateStore>,
        engines: BTreeMap<AgentId, Arc<RuntimeEngine>>,
        max_concurrency: usize,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, CookerError> {
        if engines.is_empty() {
            return Err(CookerError::InvalidConfig(
                "fleet runtime requires at least one agent route".to_owned(),
            ));
        }
        if max_concurrency == 0 {
            return Err(CookerError::InvalidConfig(
                "fleet runtime concurrency must be positive".to_owned(),
            ));
        }
        let mut signers = BTreeSet::new();
        for engine in engines.values() {
            let signer = engine.signer_address();
            if signer.trim().is_empty() {
                return Err(CookerError::InvalidConfig(
                    "fleet signer address cannot be empty".to_owned(),
                ));
            }
            if !signers.insert(signer.to_owned()) {
                return Err(CookerError::InvalidConfig(format!(
                    "fleet signer {signer} is assigned to more than one agent"
                )));
            }
        }
        let agent_gates = engines
            .keys()
            .copied()
            .map(|agent_id| (agent_id, Arc::new(Mutex::new(()))))
            .collect();
        Ok(Self {
            store,
            engines,
            agent_gates,
            clock,
            max_concurrency,
        })
    }

    /// Execute one normal lease through the engine registered for its durable agent.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown agent route or the selected engine's lifecycle failure.
    pub async fn execute(&self, lease: &ActionLease) -> Result<ExecutionResult, CookerError> {
        let (engine, gate) = self.route_for(lease)?;
        let _guard = gate.lock().await;
        engine.execute(lease).await
    }

    /// Reconcile one ambiguous lease through the same signer engine that created it.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown agent route or the selected engine's reconciliation
    /// failure. This path never submits transaction bytes.
    pub async fn reconcile(&self, lease: &ActionLease) -> Result<ExecutionResult, CookerError> {
        let (engine, gate) = self.route_for(lease)?;
        let _guard = gate.lock().await;
        engine.reconcile(lease).await
    }

    /// Audit a confirmed lease through the same signer engine that created it.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown agent route or the selected engine's observation failure.
    /// This path never submits transaction bytes.
    pub async fn audit_confirmation(
        &self,
        lease: &ActionLease,
    ) -> Result<ExecutionResult, CookerError> {
        let (engine, gate) = self.route_for(lease)?;
        let _guard = gate.lock().await;
        engine.audit_confirmation(lease).await
    }

    /// Settle a claimed normal batch with global bounded concurrency.
    pub async fn run_claimed(
        self: Arc<Self>,
        leases: Vec<ActionLease>,
        stop: watch::Receiver<bool>,
    ) -> WorkerSummary {
        self.run_batch(leases, stop, BatchMode::Execute).await
    }

    /// Settle a claimed reconciliation batch without resubmission.
    pub async fn reconcile_claimed(
        self: Arc<Self>,
        leases: Vec<ActionLease>,
        stop: watch::Receiver<bool>,
    ) -> WorkerSummary {
        self.run_batch(leases, stop, BatchMode::Reconcile).await
    }

    /// Audit a claimed confirmation batch without resubmission.
    pub async fn audit_claimed(
        self: Arc<Self>,
        leases: Vec<ActionLease>,
        stop: watch::Receiver<bool>,
    ) -> WorkerSummary {
        self.run_batch(leases, stop, BatchMode::Audit).await
    }

    fn route_for(
        &self,
        lease: &ActionLease,
    ) -> Result<(Arc<RuntimeEngine>, Arc<Mutex<()>>), CookerError> {
        let action = self.store.get_action(&lease.action_id)?;
        let engine = self.engines.get(&action.agent_id).cloned().ok_or_else(|| {
            CookerError::InvalidConfig(format!(
                "no signer runtime is registered for agent {}",
                action.agent_id
            ))
        })?;
        let gate = self
            .agent_gates
            .get(&action.agent_id)
            .cloned()
            .ok_or_else(|| {
                CookerError::InvalidConfig(format!(
                    "no execution gate is registered for agent {}",
                    action.agent_id
                ))
            })?;
        Ok((engine, gate))
    }

    async fn run_batch(
        self: Arc<Self>,
        leases: Vec<ActionLease>,
        stop: watch::Receiver<bool>,
        mode: BatchMode,
    ) -> WorkerSummary {
        tracing::info!(
            event = "worker_batch_started",
            mode = ?mode,
            claimed = leases.len(),
            max_concurrency = self.max_concurrency,
        );
        let store = Arc::clone(&self.store);
        let clock = Arc::clone(&self.clock);
        let summary = drain_bounded(
            leases,
            self.max_concurrency,
            stop,
            store.as_ref(),
            clock.as_ref(),
            |lease| {
                let fleet = Arc::clone(&self);
                async move {
                    match mode {
                        BatchMode::Execute => fleet.execute(&lease).await,
                        BatchMode::Reconcile => fleet.reconcile(&lease).await,
                        BatchMode::Audit => fleet.audit_confirmation(&lease).await,
                    }
                }
            },
        )
        .await;
        tracing::info!(
            event = "worker_batch_completed",
            mode = ?mode,
            confirmed = summary.confirmed,
            audited = summary.audited,
            orphaned = summary.orphaned,
            rejected = summary.rejected,
            delayed = summary.delayed,
            unknown = summary.unknown,
            expired = summary.expired,
            failed = summary.failed,
            errors = summary.errors.len(),
            peak_workers = summary.peak_workers,
            released_without_start = summary.released_without_start,
        );
        summary
    }
}

#[derive(Clone, Copy, Debug)]
enum BatchMode {
    Execute,
    Reconcile,
    Audit,
}
