use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use cooker_core::{
    ActionId, ActionLease, ActionState, AgentId, ChainReceipt, PlannedAction, PreparedAction,
    RunId, SimulationReceipt,
};
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

/// Durable classification assigned to a post-confirmation audit.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmationAuditOutcome {
    /// The transaction and all adapter postconditions remained confirmed.
    Verified,
    /// The prior confirmation was no longer supported by chain state.
    Orphaned,
}

/// One immutable post-confirmation observation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ConfirmationAuditRecord {
    /// Monotonic database audit identifier.
    pub id: i64,
    /// Complete observed chain receipt.
    pub receipt: ChainReceipt,
    /// Injected runtime time at which the audit was durably completed.
    pub audited_at: DateTime<Utc>,
    /// Durable audit classification.
    pub outcome: ConfirmationAuditOutcome,
}

/// Identity pinned to a database at first open.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StoreIdentity {
    /// Network family, normally `surfpool`.
    pub network: String,
    /// Unique Surfnet identifier obtained from the local network guard.
    pub surfnet_id: String,
}

/// Read-only store status evaluated at an explicit instant.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StoreStatusSnapshot {
    /// Stable database identity.
    pub database_id: Uuid,
    /// Network and Surfnet identity required when the database was opened.
    pub identity: StoreIdentity,
    /// Injected observation time used by every due and lease calculation.
    pub observed_at: DateTime<Utc>,
    /// Total durable agent records.
    pub total_agents: u64,
    /// Total durable logical actions.
    pub total_actions: u64,
    /// Counts for every lifecycle state, including explicit zeroes.
    pub actions_by_state: BTreeMap<ActionState, u64>,
    /// Unreleased leases whose expiry is later than the observation time.
    pub active_leases: u64,
    /// Unreleased leases whose expiry has elapsed and can be released by recovery.
    pub expired_leases: u64,
    /// Agent planner cursors due at or before the observation time.
    pub due_agents: u64,
    /// Normal actions currently eligible for an atomic claim.
    pub due_actions: u64,
    /// Actions in `Submitted`, `Unknown`, or `Orphaned` state.
    pub unresolved_actions: u64,
    /// Confirmed actions due before the requested audit cutoff, or `None` when auditing was not
    /// requested by the caller.
    pub due_confirmation_audits: Option<u64>,
}

/// One read-only recovery candidate for reconciliation or confirmation auditing.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RecoveryActionCandidate {
    /// Logical action identifier.
    pub action_id: ActionId,
    /// Owning run.
    pub run_id: RunId,
    /// Owning signer agent.
    pub agent_id: AgentId,
    /// Current lifecycle state.
    pub state: ActionState,
    /// Durable submission signature, when present.
    pub signature: Option<String>,
    /// Timestamp controlling deterministic recovery ordering.
    pub eligible_since: DateTime<Utc>,
}

/// One active lease whose injected expiry has elapsed.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ExpiredLeaseCandidate {
    /// Complete durable lease identity and ownership.
    pub lease: ActionLease,
    /// Time the lease was acquired.
    pub acquired_at: DateTime<Utc>,
    /// Current lifecycle state of the leased action.
    pub action_state: ActionState,
}

/// Total actionable rows in each recovery category, independent of preview truncation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RecoveryPreviewCounts {
    /// Active leases eligible for expiry release.
    pub expired_leases: u64,
    /// Unleased actions eligible for reconciliation claims.
    pub reconciliation_candidates: u64,
    /// Unleased confirmed actions eligible for audit claims, or `None` when no cutoff was given.
    pub confirmation_audit_candidates: Option<u64>,
}

/// Bounded, read-only recovery plan evaluated without releasing or claiming work.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RecoveryPreview {
    /// Injected observation time used by every eligibility check.
    pub observed_at: DateTime<Utc>,
    /// Optional strict cutoff used to select due confirmation audits.
    pub confirmation_audit_due_before: Option<DateTime<Utc>>,
    /// Maximum rows returned in each candidate list.
    pub max_items_per_category: usize,
    /// Complete category counts before preview truncation.
    pub counts: RecoveryPreviewCounts,
    /// Deterministically ordered expired lease rows.
    pub expired_leases: Vec<ExpiredLeaseCandidate>,
    /// Deterministically ordered reconciliation rows.
    pub reconciliation_candidates: Vec<RecoveryActionCandidate>,
    /// Deterministically ordered confirmation-audit rows.
    pub confirmation_audit_candidates: Vec<RecoveryActionCandidate>,
}

impl StoreIdentity {
    /// Construct a non-empty database identity.
    ///
    /// # Errors
    ///
    /// Returns an error when either identity component is empty.
    pub fn new(
        network: impl Into<String>,
        surfnet_id: impl Into<String>,
    ) -> Result<Self, cooker_core::CookerError> {
        let identity = Self {
            network: network.into(),
            surfnet_id: surfnet_id.into(),
        };
        identity.validate()?;
        Ok(identity)
    }

    /// Construct the normal local `Surfpool` identity.
    ///
    /// # Errors
    ///
    /// Returns an error when the `Surfnet` identifier is empty.
    pub fn surfpool(surfnet_id: impl Into<String>) -> Result<Self, cooker_core::CookerError> {
        Self::new("surfpool", surfnet_id)
    }

    pub(crate) fn validate(&self) -> Result<(), cooker_core::CookerError> {
        if self.network.trim().is_empty() {
            return Err(cooker_core::CookerError::InvalidConfig(
                "store network identity cannot be empty".to_owned(),
            ));
        }
        if self.surfnet_id.trim().is_empty() {
            return Err(cooker_core::CookerError::InvalidConfig(
                "store surfnet identity cannot be empty".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Complete metadata registered for a run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunRegistration {
    /// Stable run identifier.
    pub id: RunId,
    /// Planner model version.
    pub model_version: String,
    /// Sanitized run configuration.
    pub config: Value,
    /// Hash of the canonical configuration artifact.
    pub config_hash: String,
    /// Hash of the secret seed, never the seed itself.
    pub seed_hash: String,
    /// Registration timestamp.
    pub created_at: DateTime<Utc>,
}

/// Persisted run metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunRecord {
    /// Stable run identifier.
    pub id: RunId,
    /// Lifecycle status.
    pub status: String,
    /// Planner model version, absent for an automatically created stub.
    pub model_version: Option<String>,
    /// Sanitized run configuration.
    pub config: Option<Value>,
    /// Hash of the canonical configuration artifact.
    pub config_hash: Option<String>,
    /// Hash of the secret seed.
    pub seed_hash: Option<String>,
    /// Initial creation time.
    pub created_at: DateTime<Utc>,
    /// Last metadata update.
    pub updated_at: DateTime<Utc>,
}

/// One immutable event in an action journal.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionEventRecord {
    /// Monotonic database event identifier.
    pub id: i64,
    /// Stable event family.
    pub kind: String,
    /// Prior lifecycle state for transitions.
    pub from_state: Option<ActionState>,
    /// Resulting lifecycle state for transitions.
    pub to_state: Option<ActionState>,
    /// Event timestamp.
    pub occurred_at: DateTime<Utc>,
    /// Sanitized operator detail.
    pub detail: Option<String>,
    /// Optional structured payload.
    pub payload: Option<Value>,
}

/// One immutable simulation attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SimulationRecord {
    /// Simulation result.
    pub receipt: SimulationReceipt,
    /// Persistence timestamp.
    pub recorded_at: DateTime<Utc>,
}

/// The stable local signature persisted before observation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubmissionRecord {
    /// Locally derived transaction signature.
    pub signature: String,
    /// Persistence timestamp.
    pub submitted_at: DateTime<Utc>,
}

/// Complete action state required to recover after a process restart.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryRecord {
    /// Original deterministic intent.
    pub action: PlannedAction,
    /// Current lifecycle state.
    pub state: ActionState,
    /// Signed bytes, when preparation completed.
    pub prepared: Option<PreparedAction>,
    /// All distinct simulation attempts in persistence order.
    pub simulations: Vec<SimulationRecord>,
    /// Stable submission signature, when any send may have occurred.
    pub submission: Option<SubmissionRecord>,
    /// Chain observations in persistence order.
    pub receipts: Vec<ChainReceipt>,
    /// Current unexpired lease, when one exists.
    pub active_lease: Option<ActionLease>,
    /// Immutable action event journal.
    pub events: Vec<ActionEventRecord>,
}
