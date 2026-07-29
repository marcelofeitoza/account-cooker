//! Domain model and side-effect boundaries for the cooker runtime.

#![forbid(unsafe_code)]
#![allow(
    clippy::missing_errors_doc,
    reason = "all fallible domain APIs return the documented CookerError taxonomy"
)]
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        reason = "test setup uses unwrap only for values whose validity is the assertion precondition"
    )
)]

pub mod behavior;
pub mod clock;
pub mod config;
pub mod contracts;
pub mod domain;
pub mod error;
pub mod funding;
pub mod persona;
pub mod rng;
pub mod safety;
pub mod scheduler;
pub mod simulation;
pub mod trace;

pub use behavior::{ActionCatalog, BehaviorDecision, PersonaBehaviorModel, SplRoute, SwapRoute};
pub use clock::{Clock, SystemClock, VirtualClock};
pub use config::{BudgetConfig, CookerConfig, NetworkConfig, RuntimeConfig};
pub use contracts::{ActionAdapter, ChainGateway, Policy, StateStore};
pub use domain::*;
pub use error::{CookerError, ErrorClass};
pub use funding::{FundingPlan, FundingScheme, FundingTopUp, OperatorAccount, PooledFundingConfig};
pub use persona::{
    ActionWeight, ActiveWindow, AmountProfile, DurationRange, PersonaConfig, PersonaPreset,
    StateDurations, StateTransition,
};
pub use rng::{DecisionRng, derive_decision_seed};
pub use safety::{PolicyConfig, SafetyPolicy};
pub use scheduler::{Dispatch, StableScheduler};
pub use simulation::{SimulationArtifact, SimulationConfig, run_virtual_simulation};
pub use trace::{TraceEvent, TraceOutcome};
