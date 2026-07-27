//! Stable types shared by planning, persistence, execution, and evaluation.

use std::{collections::BTreeMap, fmt, time::Duration};

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::CookerError;

macro_rules! uuid_id {
    ($name:ident) => {
        #[doc = concat!("Stable ", stringify!($name), " identifier.")]
        #[derive(
            Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            /// Create a random identifier for a newly created record.
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

uuid_id!(RunId);
uuid_id!(AgentId);
uuid_id!(LeaseId);
uuid_id!(AttemptId);

const AGENT_ID_NAMESPACE: Uuid = Uuid::from_u128(0xc4b4_7e2b_4bb4_5f35_a80d_02d3_434d_8ae1);

impl AgentId {
    /// Derive a stable run-local fleet identity from a zero-based index.
    #[must_use]
    pub fn derive(run_id: RunId, index: u64) -> Self {
        let mut identity = Vec::with_capacity(24);
        identity.extend_from_slice(run_id.0.as_bytes());
        identity.extend_from_slice(&index.to_le_bytes());
        Self(Uuid::new_v5(&AGENT_ID_NAMESPACE, &identity))
    }
}

/// Deterministic logical action identifier.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ActionId(String);

impl ActionId {
    /// Derive an identifier from stable logical inputs.
    #[must_use]
    pub fn derive(run: RunId, agent: AgentId, sequence: u64, model_version: &str) -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(run.0.as_bytes());
        hasher.update(agent.0.as_bytes());
        hasher.update(&sequence.to_le_bytes());
        hasher.update(model_version.as_bytes());
        Self(hasher.finalize().to_hex().to_string())
    }

    /// Return the lowercase hexadecimal representation.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ActionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Supported logical action family.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    /// Transfer native SOL.
    NativeTransfer,
    /// Transfer an SPL token.
    SplTransfer,
    /// Swap through instructions supplied by Jupiter.
    JupiterSwap,
    /// Create, observe, and unwind a stake position.
    StakeLifecycle,
    /// Intentionally schedule no chain activity.
    Idle,
}

/// Stateful stake operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StakeOperation {
    /// Create or delegate stake.
    Enter,
    /// Deactivate an existing stake position.
    Deactivate,
    /// Withdraw a deactivated stake position.
    Withdraw,
}

/// Typed parameters for a logical action.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ActionPayload {
    /// Native SOL transfer parameters.
    NativeTransfer {
        /// Base58 destination address.
        destination: String,
        /// Amount in lamports.
        lamports: u64,
    },
    /// SPL transfer parameters.
    SplTransfer {
        /// Token mint address.
        mint: String,
        /// Destination wallet owner.
        destination_owner: String,
        /// Raw token units.
        amount: u64,
    },
    /// Jupiter exact-input swap parameters.
    JupiterSwap {
        /// Input token mint.
        input_mint: String,
        /// Output token mint.
        output_mint: String,
        /// Exact raw input units.
        amount: u64,
        /// Maximum slippage in basis points.
        max_slippage_bps: u16,
    },
    /// Stake lifecycle step.
    StakeLifecycle {
        /// Requested step.
        operation: StakeOperation,
        /// Lamports used when entering, otherwise zero.
        lamports: u64,
        /// Existing stake account for deactivate or withdraw.
        stake_account: Option<String>,
    },
    /// No chain action.
    Idle,
}

impl ActionPayload {
    /// Return the action family matching this payload.
    #[must_use]
    pub const fn kind(&self) -> ActionKind {
        match self {
            Self::NativeTransfer { .. } => ActionKind::NativeTransfer,
            Self::SplTransfer { .. } => ActionKind::SplTransfer,
            Self::JupiterSwap { .. } => ActionKind::JupiterSwap,
            Self::StakeLifecycle { .. } => ActionKind::StakeLifecycle,
            Self::Idle => ActionKind::Idle,
        }
    }

    /// Return the largest explicit asset input represented by this action.
    #[must_use]
    pub const fn input_amount(&self) -> u64 {
        match self {
            Self::NativeTransfer { lamports, .. } | Self::StakeLifecycle { lamports, .. } => {
                *lamports
            }
            Self::SplTransfer { amount, .. } | Self::JupiterSwap { amount, .. } => *amount,
            Self::Idle => 0,
        }
    }
}

/// Durable lifecycle of one logical action.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionState {
    /// Intent is persisted but not simulated.
    Planned,
    /// Transaction simulation passed.
    Simulated,
    /// A locally signed transaction may have reached the configured network.
    Submitted,
    /// Confirmation and expected state changes were observed.
    Confirmed,
    /// Policy rejected the action before submission.
    Rejected,
    /// A confirmed chain execution failed.
    Failed,
    /// The action expired before submission.
    Expired,
    /// Submission outcome is ambiguous and must be reconciled.
    Unknown,
    /// A previously observed outcome was removed by rollback.
    Orphaned,
    /// Shutdown cancelled the action before submission.
    Cancelled,
}

impl ActionState {
    /// Whether no further normal lifecycle transition is expected.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Confirmed | Self::Rejected | Self::Failed | Self::Expired | Self::Cancelled
        )
    }

    /// Validate a requested lifecycle edge.
    pub fn ensure_transition(self, next: Self) -> Result<(), CookerError> {
        let valid = matches!(
            (self, next),
            (
                Self::Planned,
                Self::Simulated | Self::Rejected | Self::Expired | Self::Cancelled
            ) | (
                Self::Simulated,
                Self::Submitted | Self::Rejected | Self::Expired | Self::Cancelled
            ) | (
                Self::Submitted,
                Self::Confirmed | Self::Failed | Self::Unknown | Self::Orphaned
            ) | (
                Self::Unknown,
                Self::Confirmed | Self::Failed | Self::Expired | Self::Orphaned
            ) | (Self::Confirmed, Self::Orphaned)
                | (
                    Self::Orphaned,
                    Self::Unknown | Self::Failed | Self::Confirmed
                )
        );
        if valid {
            Ok(())
        } else {
            Err(CookerError::InvalidTransition {
                from: self,
                to: next,
            })
        }
    }
}

/// Persisted, deterministic action intent.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PlannedAction {
    /// Deterministic logical identifier.
    pub id: ActionId,
    /// Owning run.
    pub run_id: RunId,
    /// Owning agent.
    pub agent_id: AgentId,
    /// Monotonic agent-local sequence.
    pub sequence: u64,
    /// Planner model version included in the identifier.
    pub model_version: String,
    /// Earliest execution time.
    pub scheduled_at: DateTime<Utc>,
    /// Typed action parameters.
    pub payload: ActionPayload,
    /// Maximum acceptable fee.
    pub max_fee_lamports: u64,
    /// Maximum durable lamports that may fund newly created accounts.
    pub max_account_creation_lamports: u64,
    /// Time the intent was created.
    pub created_at: DateTime<Utc>,
}

/// Snapshot supplied to a behavior model.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentSnapshot {
    /// Agent identifier.
    pub id: AgentId,
    /// Owning run.
    pub run_id: RunId,
    /// Next deterministic sequence.
    pub next_sequence: u64,
    /// Durable virtual or wall-clock cursor for the next planner decision.
    pub next_decision_at: DateTime<Utc>,
    /// UTC date on which the remaining daily budget is denominated.
    pub budget_date: NaiveDate,
    /// Current behavior state.
    pub session_state: SessionState,
    /// Last completed action timestamp.
    pub last_action_at: Option<DateTime<Utc>>,
    /// Remaining daily native-unit budget.
    pub remaining_daily_budget: u64,
    /// Planner model version.
    pub model_version: String,
}

/// Coarse semi-Markov behavior state.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    /// No activity is scheduled soon.
    Dormant,
    /// Agent is in an active session.
    Active,
    /// Agent is currently transacting.
    Transacting,
    /// Agent is holding a stateful position.
    Holding,
    /// Agent is deliberately cooling down.
    CoolingDown,
}

/// A lease that grants one worker temporary ownership of an action.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ActionLease {
    /// Lease identifier.
    pub id: LeaseId,
    /// Claimed action.
    pub action_id: ActionId,
    /// Worker identity.
    pub worker_id: String,
    /// Lease expiry.
    pub expires_at: DateTime<Utc>,
}

/// Durable native-unit spend totals used by policy checks.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct BudgetUsage {
    /// Confirmed and conservatively pending lamports in the current UTC day.
    pub spent_today_lamports: u64,
    /// Confirmed and conservatively pending lamports over the complete run.
    pub spent_lifetime_lamports: u64,
}

/// Expected post-execution state used to prove an adapter result.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StateExpectation {
    /// Stable expectation family, such as `native_balance_delta`.
    pub kind: String,
    /// Address being observed.
    pub account: String,
    /// Expected signed raw-unit delta when applicable.
    pub expected_delta: Option<i128>,
    /// Additional deterministic fields.
    pub attributes: BTreeMap<String, String>,
}

/// Locally built and signed transaction ready for simulation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PreparedAction {
    /// Logical action identifier.
    pub action_id: ActionId,
    /// Stable local signature derived during signing.
    pub signature: String,
    /// Serialized signed transaction.
    pub transaction: Vec<u8>,
    /// Blockhash used during signing.
    pub recent_blockhash: String,
    /// Last valid block height reported by the verified gateway.
    pub last_valid_block_height: u64,
    /// State changes that must be observed before success.
    pub expectations: Vec<StateExpectation>,
}

/// Result of transaction simulation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SimulationReceipt {
    /// Whether simulation completed without a transaction error.
    pub succeeded: bool,
    /// Units consumed when reported.
    pub units_consumed: Option<u64>,
    /// Sanitized program logs.
    pub logs: Vec<String>,
    /// Deterministic failure text.
    pub error: Option<String>,
}

/// Chain confirmation status independent of a Solana client implementation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmationStatus {
    /// No status has been observed.
    Missing,
    /// Transaction has been processed.
    Processed,
    /// Transaction reached confirmed commitment.
    Confirmed,
    /// Transaction reached finalized commitment.
    Finalized,
    /// Transaction failed on chain.
    Failed,
}

/// Receipt recorded after signature lookup and state observation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ChainReceipt {
    /// Logical action identifier.
    pub action_id: ActionId,
    /// Local transaction signature.
    pub signature: String,
    /// Observed confirmation status.
    pub status: ConfirmationStatus,
    /// Slot reported by the verified gateway.
    pub slot: Option<u64>,
    /// Observed time.
    pub observed_at: DateTime<Utc>,
    /// Whether all adapter expectations were proven.
    pub postconditions_met: bool,
    /// Observed state assertions.
    pub observations: Vec<StateExpectation>,
    /// Sanitized chain error when execution failed.
    pub error: Option<String>,
}

/// Context supplied to an action adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterContext {
    /// Verified gateway RPC URL.
    pub rpc_url: url::Url,
    /// Base58 signer address.
    pub signer: String,
    /// Maximum time allowed for confirmation.
    pub confirmation_timeout: Duration,
}

/// Result of a policy check.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum PolicyDecision {
    /// Execute the original action.
    Allow,
    /// Wait until the supplied time.
    Delay {
        /// Next eligible time.
        until: DateTime<Utc>,
        /// Human-readable rule identifier.
        reason: String,
    },
    /// Execute a bounded replacement.
    Rewrite {
        /// Replacement payload.
        payload: ActionPayload,
        /// Human-readable rule identifier.
        reason: String,
    },
    /// Permanently reject this action.
    Reject {
        /// Human-readable rule identifier.
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_id_is_stable_and_sensitive_to_sequence() {
        let run = RunId(Uuid::nil());
        let agent = AgentId(Uuid::from_u128(7));
        let first = ActionId::derive(run, agent, 1, "v1");
        assert_eq!(first, ActionId::derive(run, agent, 1, "v1"));
        assert_ne!(first, ActionId::derive(run, agent, 2, "v1"));
        assert_eq!(first.as_str().len(), 64);
    }

    #[test]
    fn submitted_must_reconcile_before_resubmission() {
        assert!(
            ActionState::Submitted
                .ensure_transition(ActionState::Unknown)
                .is_ok()
        );
        assert!(
            ActionState::Submitted
                .ensure_transition(ActionState::Simulated)
                .is_err()
        );
        assert!(
            ActionState::Unknown
                .ensure_transition(ActionState::Confirmed)
                .is_ok()
        );
    }
}
