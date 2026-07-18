//! Restart-safe production planning over durable agent cursors.

use std::{collections::BTreeMap, fmt, sync::Arc};

use cooker_core::{
    ActionPayload, AgentId, Clock, CookerError, DecisionRng, PersonaBehaviorModel, RunId,
    StateStore,
};

/// Counts from one fair planner pass over the currently due agents.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PlanningSummary {
    /// Durable agent decisions successfully advanced.
    pub decisions: usize,
    /// Observable actions newly inserted for execution.
    pub actions_enqueued: usize,
    /// Existing deterministic actions replayed after a crash or competing planner.
    pub actions_replayed: usize,
    /// Decisions that intentionally produced no chain action.
    pub idle_decisions: usize,
    /// Sequence compare-and-swap races lost to another planner.
    pub conflicts: usize,
}

/// Deterministic controller that advances one durable cursor per agent and pass.
pub struct PlannerFleet {
    store: Arc<dyn StateStore>,
    models: BTreeMap<AgentId, Arc<PersonaBehaviorModel>>,
    clock: Arc<dyn Clock>,
    run_id: RunId,
    global_seed: [u8; 32],
    daily_budget_lamports: u64,
}

impl fmt::Debug for PlannerFleet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PlannerFleet")
            .field("run_id", &self.run_id)
            .field("agent_count", &self.models.len())
            .field("daily_budget_lamports", &self.daily_budget_lamports)
            .finish_non_exhaustive()
    }
}

impl PlannerFleet {
    /// Construct a controller for an explicit durable run and agent-model registry.
    ///
    /// # Errors
    ///
    /// Returns an error when the registry is empty or the daily budget is zero.
    pub fn new(
        store: Arc<dyn StateStore>,
        models: BTreeMap<AgentId, Arc<PersonaBehaviorModel>>,
        clock: Arc<dyn Clock>,
        run_id: RunId,
        global_seed: [u8; 32],
        daily_budget_lamports: u64,
    ) -> Result<Self, CookerError> {
        if models.is_empty() {
            return Err(CookerError::InvalidConfig(
                "planner fleet requires at least one agent model".to_owned(),
            ));
        }
        if daily_budget_lamports == 0 {
            return Err(CookerError::InvalidConfig(
                "planner daily budget must be positive".to_owned(),
            ));
        }
        Ok(Self {
            store,
            models,
            clock,
            run_id,
            global_seed,
            daily_budget_lamports,
        })
    }

    /// Advance at most `limit` currently due agents by one deterministic decision each.
    ///
    /// Action insertion precedes the agent sequence compare-and-swap. If the process crashes in
    /// that window, the same cursor and decision seed reproduce byte-identical intent and the
    /// store's deterministic action identity turns the replay into an idempotent lookup.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid limits, missing model routes, model failures, budget overflow,
    /// or persistence failures.
    pub fn plan_due(&self, limit: usize) -> Result<PlanningSummary, CookerError> {
        let now = self.clock.now();
        let snapshots = self.store.due_agents(self.run_id, now, limit)?;
        let mut summary = PlanningSummary::default();
        for snapshot in snapshots {
            let model = self.models.get(&snapshot.id).ok_or_else(|| {
                CookerError::InvalidConfig(format!(
                    "no behavior model is registered for agent {}",
                    snapshot.id
                ))
            })?;
            let decision_at = snapshot.next_decision_at;
            if decision_at > now {
                return Err(CookerError::Store(format!(
                    "store returned agent {} before its planner cursor",
                    snapshot.id
                )));
            }
            let mut planning_snapshot = snapshot.clone();
            if planning_snapshot.budget_date != decision_at.date_naive() {
                planning_snapshot.budget_date = decision_at.date_naive();
                planning_snapshot.remaining_daily_budget = self.daily_budget_lamports;
            }
            let mut decision_rng = DecisionRng::new(
                &self.global_seed,
                planning_snapshot.id,
                planning_snapshot.next_sequence,
                &planning_snapshot.model_version,
            );
            let decision =
                model.plan_decision(&planning_snapshot, decision_at, decision_rng.rng())?;
            if decision.action.run_id != self.run_id
                || decision.action.agent_id != planning_snapshot.id
                || decision.action.sequence != planning_snapshot.next_sequence
            {
                return Err(CookerError::Store(
                    "behavior model returned an action for a different durable cursor".to_owned(),
                ));
            }

            let observable = !matches!(decision.action.payload, ActionPayload::Idle);
            let inserted = if observable {
                Some(self.store.enqueue_action(&decision.action)?)
            } else {
                None
            };
            let scheduled_date = decision.action.scheduled_at.date_naive();
            if planning_snapshot.budget_date != scheduled_date {
                planning_snapshot.budget_date = scheduled_date;
                planning_snapshot.remaining_daily_budget = self.daily_budget_lamports;
            }
            let reservation = native_reservation(&decision.action)?;
            planning_snapshot.remaining_daily_budget = planning_snapshot
                .remaining_daily_budget
                .saturating_sub(reservation);
            let expected_sequence = planning_snapshot.next_sequence;
            planning_snapshot.next_sequence =
                expected_sequence.checked_add(1).ok_or_else(|| {
                    CookerError::InvalidConfig("agent planner sequence overflow".to_owned())
                })?;
            planning_snapshot.next_decision_at = decision.action.scheduled_at;
            planning_snapshot.session_state = decision.next_state;
            planning_snapshot.last_action_at = Some(decision.action.scheduled_at);

            if !self
                .store
                .advance_agent(expected_sequence, &planning_snapshot, self.clock.now())?
            {
                summary.conflicts += 1;
                continue;
            }
            summary.decisions += 1;
            match inserted {
                Some(true) => summary.actions_enqueued += 1,
                Some(false) => summary.actions_replayed += 1,
                None => summary.idle_decisions += 1,
            }
        }
        Ok(summary)
    }
}

fn native_reservation(action: &cooker_core::PlannedAction) -> Result<u64, CookerError> {
    let asset = match action.payload {
        ActionPayload::NativeTransfer { lamports, .. }
        | ActionPayload::StakeLifecycle { lamports, .. } => lamports,
        ActionPayload::JupiterSwap { amount, .. } => amount,
        ActionPayload::SplTransfer { .. } | ActionPayload::Idle => 0,
    };
    asset
        .checked_add(action.max_fee_lamports)
        .and_then(|value| value.checked_add(action.max_account_creation_lamports))
        .ok_or_else(|| CookerError::Policy("planner native reservation overflow".to_owned()))
}
