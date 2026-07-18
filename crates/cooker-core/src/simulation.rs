//! Accelerated deterministic simulation using the production behavior and queue semantics.

use std::collections::BTreeMap;

use chrono::{DateTime, Days, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    behavior::{ActionCatalog, PersonaBehaviorModel},
    clock::{Clock, VirtualClock},
    domain::{ActionPayload, AgentId, AgentSnapshot, RunId, SessionState},
    error::CookerError,
    persona::PersonaConfig,
    rng::DecisionRng,
    scheduler::StableScheduler,
    trace::{TraceEvent, TraceOutcome},
};

const SIMULATION_NAMESPACE: Uuid = Uuid::from_u128(0xc4b4_7e2b_4bb4_5f35_a80d_02d3_434d_8ae1);

/// Bounded virtual-run configuration.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SimulationConfig {
    /// Inclusive simulation start.
    pub start: DateTime<Utc>,
    /// Number of virtual UTC days.
    pub days: u16,
    /// Number of independent wallet agents.
    pub agent_count: usize,
    /// Maximum decisions admitted in one scheduler batch.
    pub max_concurrency: usize,
    /// Maximum decisions per agent-day, guarding malformed models.
    pub max_decisions_per_agent_day: u32,
    /// Per-agent daily raw-unit budget.
    pub daily_budget: u64,
    /// Maximum fee attached to non-idle plans.
    pub max_fee_lamports: u64,
    /// Maximum account-creation funding attached to account-producing plans.
    pub max_account_creation_lamports: u64,
    /// Slippage attached to generated swaps.
    pub max_slippage_bps: u16,
    /// Stable simulation seed.
    pub global_seed: [u8; 32],
    /// Planner schema/version component.
    pub model_version: String,
}

impl SimulationConfig {
    /// Canonical scale target used by the bounty evidence artifact.
    #[must_use]
    pub fn benchmark_1000x30(start: DateTime<Utc>, global_seed: [u8; 32]) -> Self {
        Self {
            start,
            days: 30,
            agent_count: 1_000,
            max_concurrency: 64,
            max_decisions_per_agent_day: 500,
            daily_budget: 500_000_000,
            max_fee_lamports: 10_000,
            max_account_creation_lamports: 10_000_000,
            max_slippage_bps: 100,
            global_seed,
            model_version: "behavior-v1".to_owned(),
        }
    }

    /// Validate finite bounds before allocating fleet state.
    pub fn validate(&self) -> Result<(), CookerError> {
        if self.days == 0
            || self.agent_count == 0
            || self.max_concurrency == 0
            || self.max_concurrency > self.agent_count
            || self.max_decisions_per_agent_day == 0
            || self.daily_budget == 0
            || self.max_fee_lamports == 0
            || self.model_version.trim().is_empty()
        {
            return Err(CookerError::InvalidConfig(
                "simulation bounds must be positive and internally ordered".to_owned(),
            ));
        }
        if self.max_slippage_bps > 10_000 {
            return Err(CookerError::InvalidConfig(
                "simulation slippage cannot exceed 10000 basis points".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Label-free deterministic simulation output.
#[derive(Clone, Debug, PartialEq)]
pub struct SimulationArtifact {
    /// Stable run identifier.
    pub run_id: RunId,
    /// Number of simulated wallet agents.
    pub agent_count: usize,
    /// Total planner decisions, including idle transitions.
    pub decision_count: u64,
    /// Chronologically sorted observable actions only.
    pub events: Vec<TraceEvent>,
}

impl SimulationArtifact {
    /// Serialize the stable trace schema as newline-delimited JSON.
    pub fn trace_jsonl(&self) -> Result<Vec<u8>, CookerError> {
        let mut bytes = Vec::new();
        for event in &self.events {
            serde_json::to_writer(&mut bytes, event)
                .map_err(|error| CookerError::Codec(error.to_string()))?;
            bytes.push(b'\n');
        }
        Ok(bytes)
    }

    /// BLAKE3 digest of the stable trace bytes.
    pub fn trace_hash_hex(&self) -> Result<String, CookerError> {
        Ok(blake3::hash(&self.trace_jsonl()?).to_hex().to_string())
    }
}

#[derive(Debug)]
struct SimulatedAgent {
    snapshot: AgentSnapshot,
    persona_index: usize,
    budget_date: NaiveDate,
    decisions_by_day: BTreeMap<NaiveDate, u32>,
}

/// Run a fleet through a virtual clock, stable scheduler, and persona models.
#[allow(
    clippy::too_many_lines,
    reason = "the simulation loop intentionally keeps queue, clock, and state transitions visible together"
)]
pub fn run_virtual_simulation(
    config: &SimulationConfig,
    personas: &[PersonaConfig],
    catalog: &ActionCatalog,
) -> Result<SimulationArtifact, CookerError> {
    config.validate()?;
    if personas.is_empty() {
        return Err(CookerError::InvalidConfig(
            "simulation requires at least one persona".to_owned(),
        ));
    }
    for persona in personas {
        persona.validate()?;
    }
    catalog.validate()?;

    let end = config
        .start
        .checked_add_days(Days::new(u64::from(config.days)))
        .ok_or_else(|| CookerError::InvalidConfig("simulation end overflow".to_owned()))?;
    let run_id = deterministic_run_id(config);
    let models = personas
        .iter()
        .cloned()
        .map(|persona| {
            PersonaBehaviorModel::new(
                config.global_seed,
                config.model_version.clone(),
                persona,
                catalog.clone(),
                config.max_fee_lamports,
                config.max_account_creation_lamports,
                config.max_slippage_bps,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;

    let clock = VirtualClock::new(config.start);
    let mut scheduler = StableScheduler::new(config.agent_count, config.max_concurrency)?;
    let mut agents = BTreeMap::new();
    for index in 0..config.agent_count {
        let agent_index = u64::try_from(index).map_err(|error| {
            CookerError::InvalidConfig(format!("agent index does not fit u64: {error}"))
        })?;
        let agent_id = AgentId::derive(run_id, agent_index);
        scheduler.enqueue(agent_id, config.start)?;
        agents.insert(
            agent_id,
            SimulatedAgent {
                snapshot: AgentSnapshot {
                    id: agent_id,
                    run_id,
                    next_sequence: 0,
                    next_decision_at: config.start,
                    budget_date: config.start.date_naive(),
                    session_state: SessionState::Dormant,
                    last_action_at: None,
                    remaining_daily_budget: config.daily_budget,
                    model_version: config.model_version.clone(),
                },
                persona_index: index % models.len(),
                budget_date: config.start.date_naive(),
                decisions_by_day: BTreeMap::new(),
            },
        );
    }

    let mut events = Vec::new();
    let mut decision_count = 0_u64;
    while let Some(due_at) = scheduler.next_due() {
        if due_at >= end {
            break;
        }
        clock.advance_to(due_at)?;
        let dispatches = scheduler.dispatch_due(clock.now(), config.max_concurrency);
        if dispatches.is_empty() {
            return Err(CookerError::Store(
                "scheduler exposed a deadline but dispatched no work".to_owned(),
            ));
        }
        for dispatch in dispatches {
            let agent = agents.get_mut(&dispatch.agent_id).ok_or_else(|| {
                CookerError::NotFound(format!("simulated agent {}", dispatch.agent_id))
            })?;
            let day = clock.now().date_naive();
            if agent.budget_date != day {
                agent.budget_date = day;
                agent.snapshot.remaining_daily_budget = config.daily_budget;
            }
            let day_decisions = agent.decisions_by_day.entry(day).or_default();
            *day_decisions = day_decisions.checked_add(1).ok_or_else(|| {
                CookerError::InvalidConfig("daily decision count overflow".to_owned())
            })?;
            if *day_decisions > config.max_decisions_per_agent_day {
                return Err(CookerError::InvalidConfig(format!(
                    "agent {} exceeded daily decision guard",
                    dispatch.agent_id
                )));
            }

            let mut decision_rng = DecisionRng::new(
                &config.global_seed,
                agent.snapshot.id,
                agent.snapshot.next_sequence,
                &config.model_version,
            );
            let decision = models[agent.persona_index].plan_decision(
                &agent.snapshot,
                clock.now(),
                decision_rng.rng(),
            )?;
            if decision.action.scheduled_at <= clock.now() {
                return Err(CookerError::InvalidConfig(
                    "behavior model scheduled a non-future decision".to_owned(),
                ));
            }
            if decision.action.scheduled_at < end
                && !matches!(decision.action.payload, ActionPayload::Idle)
            {
                events.push(trace_event(&decision.action));
            }
            let scheduled_day = decision.action.scheduled_at.date_naive();
            if agent.budget_date != scheduled_day {
                agent.budget_date = scheduled_day;
                agent.snapshot.remaining_daily_budget = config.daily_budget;
            }
            let amount = native_budget_amount(&decision.action.payload);
            agent.snapshot.remaining_daily_budget =
                agent.snapshot.remaining_daily_budget.saturating_sub(amount);
            agent.snapshot.next_sequence =
                agent.snapshot.next_sequence.checked_add(1).ok_or_else(|| {
                    CookerError::InvalidConfig("agent sequence overflow".to_owned())
                })?;
            agent.snapshot.next_decision_at = decision.action.scheduled_at;
            agent.snapshot.budget_date = scheduled_day;
            agent.snapshot.session_state = decision.next_state;
            agent.snapshot.last_action_at = Some(decision.action.scheduled_at);
            decision_count = decision_count
                .checked_add(1)
                .ok_or_else(|| CookerError::InvalidConfig("decision count overflow".to_owned()))?;
            scheduler.complete(dispatch.agent_id, Some(decision.action.scheduled_at))?;
        }
    }
    scheduler.cancel();
    events.sort_by(|left, right| {
        left.scheduled_at
            .cmp(&right.scheduled_at)
            .then_with(|| left.agent_id.cmp(&right.agent_id))
            .then_with(|| left.sequence.cmp(&right.sequence))
    });
    Ok(SimulationArtifact {
        run_id,
        agent_count: config.agent_count,
        decision_count,
        events,
    })
}

fn deterministic_run_id(config: &SimulationConfig) -> RunId {
    let mut identity = Vec::with_capacity(80);
    identity.extend_from_slice(&config.global_seed);
    identity.extend_from_slice(&config.start.timestamp_millis().to_le_bytes());
    identity.extend_from_slice(&config.days.to_le_bytes());
    identity.extend_from_slice(&config.agent_count.to_le_bytes());
    identity.extend_from_slice(config.model_version.as_bytes());
    RunId(Uuid::new_v5(&SIMULATION_NAMESPACE, &identity))
}

fn trace_event(action: &crate::domain::PlannedAction) -> TraceEvent {
    TraceEvent {
        schema_version: TraceEvent::SCHEMA_VERSION,
        run_id: action.run_id,
        agent_id: action.agent_id,
        action_id: action.id.clone(),
        sequence: action.sequence,
        action_kind: action.payload.kind(),
        scheduled_at: action.scheduled_at,
        observed_at: None,
        amount: action.payload.input_amount(),
        destination: observable_destination(&action.payload),
        signature: None,
        outcome: TraceOutcome::Planned,
        attributes: BTreeMap::new(),
    }
}

fn observable_destination(payload: &ActionPayload) -> Option<String> {
    match payload {
        ActionPayload::NativeTransfer { destination, .. } => Some(destination.clone()),
        ActionPayload::SplTransfer {
            destination_owner, ..
        } => Some(destination_owner.clone()),
        ActionPayload::JupiterSwap { .. } => Some("jupiter".to_owned()),
        ActionPayload::StakeLifecycle { .. } => Some("native_stake".to_owned()),
        ActionPayload::Idle => None,
    }
}

fn native_budget_amount(payload: &ActionPayload) -> u64 {
    match payload {
        ActionPayload::NativeTransfer { lamports, .. }
        | ActionPayload::StakeLifecycle { lamports, .. } => *lamports,
        ActionPayload::JupiterSwap { amount, .. } => *amount,
        ActionPayload::SplTransfer { .. } | ActionPayload::Idle => 0,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use chrono::TimeZone;

    use super::*;
    use crate::{
        behavior::{SplRoute, SwapRoute},
        domain::ActionKind,
        persona::{PersonaConfig, PersonaPreset},
    };

    fn catalog() -> ActionCatalog {
        ActionCatalog {
            native_destinations: (0..8).map(|index| format!("destination-{index}")).collect(),
            spl_routes: vec![SplRoute {
                mint: "mint-c".to_owned(),
                destination_owner: "destination-0".to_owned(),
            }],
            swap_routes: vec![
                SwapRoute {
                    input_mint: "mint-a".to_owned(),
                    output_mint: "mint-b".to_owned(),
                },
                SwapRoute {
                    input_mint: "mint-b".to_owned(),
                    output_mint: "mint-a".to_owned(),
                },
            ],
            allow_stake_enter: false,
        }
    }

    fn personas() -> Vec<PersonaConfig> {
        [
            PersonaPreset::Casual,
            PersonaPreset::Trader,
            PersonaPreset::Saver,
            PersonaPreset::Explorer,
        ]
        .into_iter()
        .map(PersonaConfig::preset)
        .collect()
    }

    #[test]
    fn small_runs_are_byte_identical_and_label_free() {
        let mut config = SimulationConfig::benchmark_1000x30(
            Utc.with_ymd_and_hms(2026, 7, 17, 0, 0, 0).unwrap(),
            [42; 32],
        );
        config.agent_count = 24;
        config.days = 3;
        config.max_concurrency = 8;
        let first = run_virtual_simulation(&config, &personas(), &catalog()).unwrap();
        let second = run_virtual_simulation(&config, &personas(), &catalog()).unwrap();
        let left = first.trace_jsonl().unwrap();
        assert_eq!(left, second.trace_jsonl().unwrap());
        let text = String::from_utf8(left).unwrap();
        assert!(!text.contains("controller"));
        assert!(!text.contains("persona"));
        assert!(!first.events.is_empty());
    }

    #[test]
    fn canonical_thousand_agent_month_has_a_stable_trace() {
        let config = SimulationConfig::benchmark_1000x30(
            Utc.with_ymd_and_hms(2026, 7, 17, 0, 0, 0).unwrap(),
            [42; 32],
        );
        let artifact = run_virtual_simulation(&config, &personas(), &catalog()).unwrap();
        assert_eq!(artifact.agent_count, 1_000);
        assert!(!artifact.events.is_empty());
        assert_eq!(
            artifact.trace_hash_hex().unwrap(),
            "7f6b46f889996b6a793788a2ab510b166fd965f605aa27b6dbc0dddcdb8fbaf4"
        );
        assert_eq!(
            (artifact.decision_count, artifact.events.len()),
            (364_128, 102_558)
        );
        assert!(artifact.decision_count > artifact.events.len() as u64);
    }

    #[test]
    fn trace_is_chronological_and_contains_only_observable_actions() {
        let mut config = SimulationConfig::benchmark_1000x30(
            Utc.with_ymd_and_hms(2026, 7, 17, 0, 0, 0).unwrap(),
            [7; 32],
        );
        config.agent_count = 12;
        config.max_concurrency = 4;
        config.days = 2;
        let artifact = run_virtual_simulation(&config, &personas(), &catalog()).unwrap();
        assert!(artifact.events.windows(2).all(|pair| {
            (pair[0].scheduled_at, pair[0].agent_id, pair[0].sequence)
                <= (pair[1].scheduled_at, pair[1].agent_id, pair[1].sequence)
        }));
        assert!(
            artifact
                .events
                .iter()
                .all(|event| event.action_kind != ActionKind::Idle)
        );
    }

    #[test]
    fn invalid_or_runaway_configuration_is_rejected() {
        let mut config = SimulationConfig::benchmark_1000x30(Utc::now(), [0; 32]);
        config.max_concurrency = 1_001;
        assert!(run_virtual_simulation(&config, &personas(), &catalog()).is_err());
    }

    #[test]
    fn all_trace_agent_ids_belong_to_the_requested_fleet() {
        let mut config = SimulationConfig::benchmark_1000x30(
            Utc.with_ymd_and_hms(2026, 7, 17, 0, 0, 0).unwrap(),
            [11; 32],
        );
        config.agent_count = 16;
        config.max_concurrency = 4;
        config.days = 2;
        let artifact = run_virtual_simulation(&config, &personas(), &catalog()).unwrap();
        let unique: BTreeSet<_> = artifact.events.iter().map(|event| event.agent_id).collect();
        assert!(unique.len() <= 16);
    }
}
