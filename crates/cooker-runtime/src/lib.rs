//! Bounded orchestration, fault injection, and restart reconciliation.

#![forbid(unsafe_code)]

mod engine;
mod fault;
mod fleet;
mod planner;
mod workers;

pub use engine::{ExecutionResult, RuntimeEngine, RuntimeSettings};
pub use fault::{ExecutionCheckpoint, FaultInjector, NoFaults};
pub use fleet::FleetRuntime;
pub use planner::{PlannerFleet, PlanningSummary};
pub use workers::WorkerSummary;
