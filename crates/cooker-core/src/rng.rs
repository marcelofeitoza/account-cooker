//! Reproducible decision-level random number generation.

use rand_chacha::{ChaCha12Rng, rand_core::SeedableRng};

use crate::domain::AgentId;

/// Seed a decision from independent stable inputs.
#[must_use]
pub fn derive_decision_seed(
    global_seed: &[u8; 32],
    agent_id: AgentId,
    sequence: u64,
    model_version: &str,
) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(global_seed);
    hasher.update(agent_id.0.as_bytes());
    hasher.update(&sequence.to_le_bytes());
    hasher.update(model_version.as_bytes());
    *hasher.finalize().as_bytes()
}

/// Deterministic RNG used for exactly one planner decision.
#[derive(Debug)]
pub struct DecisionRng(ChaCha12Rng);

impl DecisionRng {
    /// Construct the RNG for one agent decision.
    #[must_use]
    pub fn new(
        global_seed: &[u8; 32],
        agent_id: AgentId,
        sequence: u64,
        model_version: &str,
    ) -> Self {
        Self(ChaCha12Rng::from_seed(derive_decision_seed(
            global_seed,
            agent_id,
            sequence,
            model_version,
        )))
    }

    /// Borrow the underlying deterministic generator.
    pub const fn rng(&mut self) -> &mut ChaCha12Rng {
        &mut self.0
    }
}

#[cfg(test)]
mod tests {
    use rand_chacha::rand_core::RngCore;
    use uuid::Uuid;

    use super::*;

    #[test]
    fn fixed_inputs_produce_identical_streams() {
        let agent = AgentId(Uuid::from_u128(99));
        let mut left = DecisionRng::new(&[7; 32], agent, 4, "model-v1");
        let mut right = DecisionRng::new(&[7; 32], agent, 4, "model-v1");
        assert_eq!(left.rng().next_u64(), right.rng().next_u64());
        assert_eq!(left.rng().next_u64(), right.rng().next_u64());
    }
}
