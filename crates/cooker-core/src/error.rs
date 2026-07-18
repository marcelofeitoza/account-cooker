//! Shared error categories used for retry and recovery decisions.

use thiserror::Error;

/// Operational classification that controls whether an action may be retried.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorClass {
    /// Input, account, or transaction data is deterministically invalid.
    Deterministic,
    /// A short-lived failure is safe to retry before submission.
    Transient,
    /// Policy intentionally blocked the action.
    Policy,
    /// Submission may have reached the chain and requires reconciliation.
    UnknownOutcome,
    /// Stored state or a state transition violated an invariant.
    Invariant,
}

/// Error shared across the pure domain interfaces.
#[derive(Debug, Error)]
pub enum CookerError {
    /// Configuration is internally inconsistent or unsafe.
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),
    /// A requested lifecycle transition is not legal.
    #[error("invalid action transition from {from:?} to {to:?}")]
    InvalidTransition {
        /// Current state.
        from: crate::domain::ActionState,
        /// Requested state.
        to: crate::domain::ActionState,
    },
    /// A policy rule rejected execution.
    #[error("policy rejected action: {0}")]
    Policy(String),
    /// A durable record could not be found.
    #[error("record not found: {0}")]
    NotFound(String),
    /// A lease is held by another worker or has expired.
    #[error("lease conflict: {0}")]
    LeaseConflict(String),
    /// A storage operation failed.
    #[error("store error: {0}")]
    Store(String),
    /// A chain-facing operation failed before an outcome became ambiguous.
    #[error("chain error: {0}")]
    Chain(String),
    /// A send may have landed and must be reconciled before any retry.
    #[error("unknown submission outcome: {0}")]
    UnknownOutcome(String),
    /// Serialization or parsing failed.
    #[error("codec error: {0}")]
    Codec(String),
}

impl CookerError {
    /// Return the operational class used by the runtime retry policy.
    #[must_use]
    pub const fn class(&self) -> ErrorClass {
        match self {
            Self::InvalidConfig(_) | Self::NotFound(_) | Self::Codec(_) => {
                ErrorClass::Deterministic
            }
            Self::InvalidTransition { .. } | Self::Store(_) => ErrorClass::Invariant,
            Self::Policy(_) => ErrorClass::Policy,
            Self::LeaseConflict(_) | Self::Chain(_) => ErrorClass::Transient,
            Self::UnknownOutcome(_) => ErrorClass::UnknownOutcome,
        }
    }
}
