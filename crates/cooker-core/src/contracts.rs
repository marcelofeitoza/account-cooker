//! Side-effect boundaries implemented by the store, runtime, and chain crates.

use chrono::{DateTime, Utc};

use crate::{
    domain::{
        ActionId, ActionLease, ActionPayload, ActionState, AdapterContext, AgentSnapshot,
        BudgetUsage, ChainReceipt, PlannedAction, PolicyDecision, PreparedAction, RunId,
        SimulationReceipt,
    },
    error::CookerError,
    trace::TraceEvent,
};

/// Pure pre-execution safety policy.
pub trait Policy: Send + Sync {
    /// Evaluate an action against current observed balances and budgets.
    fn evaluate(
        &self,
        action: &PlannedAction,
        signer_balance: u64,
        spent_today: u64,
        spent_lifetime: u64,
    ) -> Result<PolicyDecision, CookerError>;
}

/// Durable state boundary with lease and idempotency semantics.
pub trait StateStore: Send + Sync {
    /// Insert or update an agent during explicit fleet initialization.
    fn upsert_agent(&self, snapshot: &AgentSnapshot, at: DateTime<Utc>) -> Result<(), CookerError>;
    /// Load one durable agent planner snapshot.
    fn get_agent(&self, agent_id: crate::domain::AgentId) -> Result<AgentSnapshot, CookerError>;
    /// Return due agent snapshots in stable planner order.
    fn due_agents(
        &self,
        run_id: RunId,
        now: DateTime<Utc>,
        limit: usize,
    ) -> Result<Vec<AgentSnapshot>, CookerError>;
    /// Advance an agent only when its durable sequence still matches the caller's snapshot.
    fn advance_agent(
        &self,
        expected_sequence: u64,
        snapshot: &AgentSnapshot,
        at: DateTime<Utc>,
    ) -> Result<bool, CookerError>;
    /// Enqueue a logical action once, returning false when it already exists.
    fn enqueue_action(&self, action: &PlannedAction) -> Result<bool, CookerError>;
    /// Atomically claim due work.
    fn claim_due_actions(
        &self,
        worker_id: &str,
        now: DateTime<Utc>,
        lease_until: DateTime<Utc>,
        limit: usize,
    ) -> Result<Vec<ActionLease>, CookerError>;
    /// Load a planned action by identifier.
    fn get_action(&self, action_id: &ActionId) -> Result<PlannedAction, CookerError>;
    /// Load the current durable lifecycle state.
    fn get_action_state(&self, action_id: &ActionId) -> Result<ActionState, CookerError>;
    /// Load the exact signed transaction persisted before submission.
    fn get_prepared(&self, action_id: &ActionId) -> Result<Option<PreparedAction>, CookerError>;
    /// Load the stable signature recorded for a submission attempt.
    fn get_submission_signature(&self, action_id: &ActionId)
    -> Result<Option<String>, CookerError>;
    /// Read durable native-unit usage for restart-safe policy enforcement.
    fn budget_usage(
        &self,
        agent_id: crate::domain::AgentId,
        now: DateTime<Utc>,
    ) -> Result<BudgetUsage, CookerError>;
    /// Release an owned lease after a worker stops processing it.
    fn release_lease(&self, lease: &ActionLease, at: DateTime<Utc>) -> Result<(), CookerError>;
    /// Persist a policy deferral and release the owned lease atomically.
    fn defer_action(
        &self,
        lease: &ActionLease,
        until: DateTime<Utc>,
        at: DateTime<Utc>,
        reason: &str,
    ) -> Result<(), CookerError>;
    /// Atomically replace the payload of an unprepared planned action and journal both intents.
    fn rewrite_action(
        &self,
        lease: &ActionLease,
        payload: &ActionPayload,
        at: DateTime<Utc>,
        reason: &str,
    ) -> Result<PlannedAction, CookerError>;
    /// Persist a validated lifecycle transition and its immutable event.
    fn transition_action(
        &self,
        lease: &ActionLease,
        expected: ActionState,
        next: ActionState,
        at: DateTime<Utc>,
        detail: Option<&str>,
    ) -> Result<(), CookerError>;
    /// Persist a lifecycle transition and its evaluator trace in one atomic operation.
    ///
    /// Implementations must commit both records or neither so a crash cannot leave a terminal
    /// action without its corresponding observation trace.
    fn transition_action_with_trace(
        &self,
        lease: &ActionLease,
        expected: ActionState,
        next: ActionState,
        at: DateTime<Utc>,
        detail: Option<&str>,
        event: &TraceEvent,
    ) -> Result<(), CookerError>;
    /// Persist signed bytes before any submission attempt.
    fn record_prepared(
        &self,
        lease: &ActionLease,
        prepared: &PreparedAction,
        at: DateTime<Utc>,
    ) -> Result<(), CookerError>;
    /// Persist a simulation receipt.
    fn record_simulation(
        &self,
        lease: &ActionLease,
        receipt: &SimulationReceipt,
        at: DateTime<Utc>,
    ) -> Result<(), CookerError>;
    /// Persist the stable signature before waiting for confirmation.
    fn record_submission(
        &self,
        lease: &ActionLease,
        signature: &str,
        at: DateTime<Utc>,
    ) -> Result<(), CookerError>;
    /// Persist observation and terminal receipt.
    fn record_receipt(
        &self,
        lease: &ActionLease,
        receipt: &ChainReceipt,
    ) -> Result<(), CookerError>;
    /// Atomically claim ambiguous or submitted actions for reconciliation.
    fn claim_reconciliation_candidates(
        &self,
        worker_id: &str,
        now: DateTime<Utc>,
        lease_until: DateTime<Utc>,
        limit: usize,
    ) -> Result<Vec<ActionLease>, CookerError>;
    /// Atomically claim confirmed actions whose latest confirmation observation is before the
    /// supplied audit cutoff.
    fn claim_confirmation_audit_candidates(
        &self,
        worker_id: &str,
        now: DateTime<Utc>,
        lease_until: DateTime<Utc>,
        due_before: DateTime<Utc>,
        limit: usize,
    ) -> Result<Vec<ActionLease>, CookerError>;
    /// Atomically persist a successful confirmation audit and release its owned lease.
    fn complete_confirmation_audit(
        &self,
        lease: &ActionLease,
        receipt: &ChainReceipt,
        audited_at: DateTime<Utc>,
    ) -> Result<(), CookerError>;
    /// Atomically persist a downgraded confirmation, correct it to orphaned, append its trace,
    /// and release its owned lease.
    fn orphan_confirmation(
        &self,
        lease: &ActionLease,
        receipt: &ChainReceipt,
        audited_at: DateTime<Utc>,
        reason: &str,
        event: &TraceEvent,
    ) -> Result<(), CookerError>;
    /// Append a stable trace event.
    fn append_trace(&self, event: &TraceEvent) -> Result<(), CookerError>;
}

/// Chain gateway restricted by its implementation to a verified network endpoint.
#[async_trait::async_trait]
pub trait ChainGateway: Send + Sync {
    /// Simulate a signed transaction without submitting it.
    async fn simulate(&self, transaction: &[u8]) -> Result<SimulationReceipt, CookerError>;
    /// Submit signed bytes, returning the deterministic transaction signature.
    async fn submit(&self, transaction: &[u8]) -> Result<String, CookerError>;
    /// Look up and observe a transaction by signature.
    async fn observe(
        &self,
        action_id: &ActionId,
        signature: &str,
    ) -> Result<ChainReceipt, CookerError>;
    /// Read a native account balance in lamports.
    async fn native_balance(&self, address: &str) -> Result<u64, CookerError>;
    /// Read the current network block height for expiry reconciliation.
    async fn block_height(&self) -> Result<u64, CookerError>;
}

/// Complete adapter boundary for one typed action family.
#[async_trait::async_trait]
pub trait ActionAdapter: Send + Sync {
    /// Whether this adapter accepts the supplied payload.
    fn supports(&self, payload: &ActionPayload) -> bool;
    /// Build and sign locally using a blockhash fetched from the verified gateway.
    async fn prepare(
        &self,
        context: &AdapterContext,
        action: &PlannedAction,
    ) -> Result<PreparedAction, CookerError>;
    /// Prove adapter-specific postconditions after confirmation.
    async fn observe(
        &self,
        context: &AdapterContext,
        action: &PlannedAction,
        prepared: &PreparedAction,
    ) -> Result<ChainReceipt, CookerError>;
}
