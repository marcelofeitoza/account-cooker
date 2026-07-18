//! Separate containers for public observations and private evaluation labels.

use std::collections::{BTreeMap, BTreeSet};

use cooker_core::{AgentId, TraceEvent};
use serde::{Deserialize, Serialize};

/// Label-free input accepted by feature extraction.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ObservationDataset {
    /// Stable trace rows.
    pub events: Vec<TraceEvent>,
}

impl ObservationDataset {
    /// Sort rows into a stable serialization order.
    pub fn normalize(&mut self) {
        self.events.sort_by(|left, right| {
            left.agent_id
                .cmp(&right.agent_id)
                .then(left.sequence.cmp(&right.sequence))
                .then(left.action_id.cmp(&right.action_id))
        });
    }

    /// Return every agent represented by the observations.
    #[must_use]
    pub fn agents(&self) -> BTreeSet<AgentId> {
        self.events.iter().map(|event| event.agent_id).collect()
    }
}

/// Controller labels supplied only to metric calculation.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct GroundTruth {
    /// Agent-to-controller mapping.
    pub controllers: BTreeMap<AgentId, String>,
}

impl GroundTruth {
    /// Return whether two agents share a known controller.
    #[must_use]
    pub fn same_controller(&self, left: AgentId, right: AgentId) -> Option<bool> {
        let left_label = self.controllers.get(&left)?;
        let right_label = self.controllers.get(&right)?;
        Some(left_label == right_label)
    }

    /// Validate that labels cover exactly the evaluated agents.
    ///
    /// # Errors
    ///
    /// Returns an error when labels omit an observed agent or contain an unobserved agent.
    pub fn validate_agents(&self, agents: &BTreeSet<AgentId>) -> Result<(), String> {
        let labelled: BTreeSet<_> = self.controllers.keys().copied().collect();
        if &labelled == agents {
            Ok(())
        } else {
            Err("ground-truth labels must cover exactly the observed agents".to_owned())
        }
    }
}
