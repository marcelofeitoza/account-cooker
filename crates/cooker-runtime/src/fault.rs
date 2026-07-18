//! Deterministic lifecycle fault injection used by recovery proofs.

use cooker_core::CookerError;

/// Stable checkpoints spanning persistence, signing, submission, and promotion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionCheckpoint {
    /// Intent exists before this runtime claims it.
    AfterIntentPersistence,
    /// Signed bytes and signature are durable but simulation is not recorded.
    AfterPreparedPersistence,
    /// Simulation passed and state is `Simulated`.
    AfterSimulation,
    /// Signature is recorded and state is `Submitted`, before send.
    AfterSignaturePersistence,
    /// Send returned but the response is deliberately treated as lost.
    AfterSendResponseLost,
    /// Receipt is durable but lifecycle promotion is not.
    AfterConfirmationBeforePromotion,
}

/// Hook that can stop execution at a named recovery checkpoint.
pub trait FaultInjector: Send + Sync {
    /// Return an injected error when execution should stop here.
    ///
    /// # Errors
    ///
    /// Returns the deliberate injected fault for the selected checkpoint.
    fn check(&self, checkpoint: ExecutionCheckpoint) -> Result<(), CookerError>;
}

/// Production injector that never introduces faults.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoFaults;

impl FaultInjector for NoFaults {
    fn check(&self, _checkpoint: ExecutionCheckpoint) -> Result<(), CookerError> {
        Ok(())
    }
}
