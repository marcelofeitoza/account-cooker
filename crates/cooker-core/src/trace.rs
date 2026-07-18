//! Stable observation-only trace schema consumed by the evaluator.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::domain::{ActionId, ActionKind, AgentId, RunId};

/// Public outcome category recorded without controller labels.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceOutcome {
    /// Action passed policy and planning only.
    Planned,
    /// Action landed and its postconditions were observed.
    Confirmed,
    /// Action was intentionally rejected.
    Rejected,
    /// Action failed or expired.
    Failed,
}

/// One evaluator input row. Controller identity is deliberately absent.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TraceEvent {
    /// Schema version for artifact compatibility.
    pub schema_version: u16,
    /// Run identifier.
    pub run_id: RunId,
    /// Pseudonymous agent identifier.
    pub agent_id: AgentId,
    /// Logical action identifier.
    pub action_id: ActionId,
    /// Agent-local sequence.
    pub sequence: u64,
    /// Logical action family.
    pub action_kind: ActionKind,
    /// Scheduled timestamp.
    pub scheduled_at: DateTime<Utc>,
    /// Observed timestamp, absent for planned traces.
    pub observed_at: Option<DateTime<Utc>>,
    /// Raw input amount.
    pub amount: u64,
    /// Public destination or protocol identifier.
    pub destination: Option<String>,
    /// Local transaction signature when executed.
    pub signature: Option<String>,
    /// Outcome category.
    pub outcome: TraceOutcome,
    /// Stable observable attributes only.
    pub attributes: BTreeMap<String, String>,
}

impl TraceEvent {
    /// Current stable schema revision.
    pub const SCHEMA_VERSION: u16 = 1;
}
