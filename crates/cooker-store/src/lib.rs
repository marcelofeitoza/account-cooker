//! Durable `SQLite` implementation of the cooker state contract.
//!
//! The store uses one mutex-protected `SQLite` connection per [`Store`] and
//! `BEGIN IMMEDIATE` transactions for claims and compare-and-swap updates.
//! Separate `Store` instances may safely coordinate through the same WAL file.

#![forbid(unsafe_code)]

mod migration;
mod models;
mod store;

pub use models::{
    ActionEventRecord, ConfirmationAuditOutcome, ConfirmationAuditRecord, ExpiredLeaseCandidate,
    RecoveryActionCandidate, RecoveryPreview, RecoveryPreviewCounts, RecoveryRecord, RunRecord,
    RunRegistration, SimulationRecord, StoreIdentity, StoreStatusSnapshot, SubmissionRecord,
};
pub use store::Store;

#[cfg(test)]
mod tests;
