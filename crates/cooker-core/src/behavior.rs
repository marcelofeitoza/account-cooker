//! Deterministic semi-Markov planning over validated personas.

use chrono::{DateTime, Datelike, Days, NaiveDate, TimeDelta, Timelike, Utc};
use rand::Rng;
use rand_chacha::ChaCha12Rng;
use serde::{Deserialize, Serialize};

use crate::{
    contracts::BehaviorModel,
    domain::{
        ActionId, ActionKind, ActionPayload, AgentSnapshot, PlannedAction, SessionState,
        StakeOperation,
    },
    error::CookerError,
    persona::{ActiveWindow, PersonaConfig},
};

const BASIS_POINTS: u64 = 10_000;
const ELIGIBILITY_SEARCH_DAYS: u64 = 370;

/// One configured SPL-token movement route.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SplRoute {
    /// Token mint.
    pub mint: String,
    /// Destination owner.
    pub destination_owner: String,
}

/// One configured exact-input Jupiter route.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SwapRoute {
    /// Input mint.
    pub input_mint: String,
    /// Output mint.
    pub output_mint: String,
}

/// Operator-owned destinations and protocol routes available to a planner.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ActionCatalog {
    /// Allowed native-transfer destinations.
    pub native_destinations: Vec<String>,
    /// Allowed SPL routes.
    pub spl_routes: Vec<SplRoute>,
    /// Allowed Jupiter pairs.
    pub swap_routes: Vec<SwapRoute>,
    /// Whether the planner may enter a native stake position.
    #[serde(default)]
    pub allow_stake_enter: bool,
}

impl ActionCatalog {
    /// Validate that configured values are non-empty and swap pairs differ.
    pub fn validate(&self) -> Result<(), CookerError> {
        if self
            .native_destinations
            .iter()
            .any(|destination| destination.trim().is_empty())
        {
            return Err(CookerError::InvalidConfig(
                "native destination cannot be empty".to_owned(),
            ));
        }
        if self
            .spl_routes
            .iter()
            .any(|route| route.mint.trim().is_empty() || route.destination_owner.trim().is_empty())
        {
            return Err(CookerError::InvalidConfig(
                "SPL routes require a mint and destination".to_owned(),
            ));
        }
        if self.swap_routes.iter().any(|route| {
            route.input_mint.trim().is_empty()
                || route.output_mint.trim().is_empty()
                || route.input_mint == route.output_mint
        }) {
            return Err(CookerError::InvalidConfig(
                "swap routes require distinct non-empty mints".to_owned(),
            ));
        }
        Ok(())
    }

    fn supports(&self, kind: ActionKind) -> bool {
        match kind {
            ActionKind::NativeTransfer => !self.native_destinations.is_empty(),
            ActionKind::SplTransfer => !self.spl_routes.is_empty(),
            ActionKind::JupiterSwap => !self.swap_routes.is_empty(),
            ActionKind::StakeLifecycle => self.allow_stake_enter,
            ActionKind::Idle => true,
        }
    }
}

/// Rich result exposed to runtimes that persist semi-Markov state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BehaviorDecision {
    /// Deterministic logical action, including idle transitions.
    pub action: PlannedAction,
    /// State to persist before the next planner decision.
    pub next_state: SessionState,
}

/// Deterministic persona model using exactly one caller-owned RNG per decision.
#[derive(Clone, Debug)]
pub struct PersonaBehaviorModel {
    global_seed: [u8; 32],
    model_version: String,
    persona: PersonaConfig,
    catalog: ActionCatalog,
    max_fee_lamports: u64,
    max_account_creation_lamports: u64,
    max_slippage_bps: u16,
}

impl PersonaBehaviorModel {
    /// Construct a model after validating every pure configuration invariant.
    pub fn new(
        global_seed: [u8; 32],
        model_version: impl Into<String>,
        persona: PersonaConfig,
        catalog: ActionCatalog,
        max_fee_lamports: u64,
        max_account_creation_lamports: u64,
        max_slippage_bps: u16,
    ) -> Result<Self, CookerError> {
        persona.validate()?;
        catalog.validate()?;
        let model_version = model_version.into();
        if model_version.trim().is_empty() {
            return Err(CookerError::InvalidConfig(
                "model version cannot be empty".to_owned(),
            ));
        }
        if max_fee_lamports == 0 {
            return Err(CookerError::InvalidConfig(
                "maximum fee must be positive".to_owned(),
            ));
        }
        if max_slippage_bps > 10_000 {
            return Err(CookerError::InvalidConfig(
                "maximum slippage cannot exceed 10000 basis points".to_owned(),
            ));
        }
        Ok(Self {
            global_seed,
            model_version,
            persona,
            catalog,
            max_fee_lamports,
            max_account_creation_lamports,
            max_slippage_bps,
        })
    }

    /// Return this model's validated persona.
    #[must_use]
    pub const fn persona(&self) -> &PersonaConfig {
        &self.persona
    }

    /// Plan one transition and the action associated with it.
    pub fn plan_decision(
        &self,
        agent: &AgentSnapshot,
        now: DateTime<Utc>,
        rng: &mut ChaCha12Rng,
    ) -> Result<BehaviorDecision, CookerError> {
        if agent.model_version != self.model_version {
            return Err(CookerError::InvalidConfig(format!(
                "agent model version {} does not match {}",
                agent.model_version, self.model_version
            )));
        }

        if !self.is_eligible_now(agent, now) {
            let scheduled_at = self.next_eligible_time(agent, now)?;
            return Ok(self.decision(
                agent,
                now,
                scheduled_at,
                SessionState::Dormant,
                ActionPayload::Idle,
            ));
        }

        let next_state = self.choose_transition(agent.session_state, rng)?;
        let dwell = self.sample_dwell(next_state, rng)?;
        let scheduled_at = now
            .checked_add_signed(TimeDelta::seconds(dwell))
            .ok_or_else(|| CookerError::InvalidConfig("scheduled time overflow".to_owned()))?;

        if !self.is_eligible_now(agent, scheduled_at) {
            let next = self.next_eligible_time(agent, now)?;
            return Ok(self.decision(agent, now, next, SessionState::Dormant, ActionPayload::Idle));
        }

        let payload = if next_state == SessionState::Transacting {
            self.choose_payload(agent.remaining_daily_budget, rng)?
        } else {
            ActionPayload::Idle
        };
        let effective_state =
            if matches!(payload, ActionPayload::Idle) && next_state == SessionState::Transacting {
                SessionState::CoolingDown
            } else {
                next_state
            };
        Ok(self.decision(agent, now, scheduled_at, effective_state, payload))
    }

    fn decision(
        &self,
        agent: &AgentSnapshot,
        now: DateTime<Utc>,
        scheduled_at: DateTime<Utc>,
        next_state: SessionState,
        payload: ActionPayload,
    ) -> BehaviorDecision {
        let max_fee_lamports = if matches!(payload, ActionPayload::Idle) {
            0
        } else {
            self.max_fee_lamports
        };
        let max_account_creation_lamports = if matches!(
            payload,
            ActionPayload::SplTransfer { .. }
                | ActionPayload::JupiterSwap { .. }
                | ActionPayload::StakeLifecycle { .. }
        ) {
            self.max_account_creation_lamports
        } else {
            0
        };
        BehaviorDecision {
            action: PlannedAction {
                id: ActionId::derive(
                    agent.run_id,
                    agent.id,
                    agent.next_sequence,
                    &self.model_version,
                ),
                run_id: agent.run_id,
                agent_id: agent.id,
                sequence: agent.next_sequence,
                model_version: self.model_version.clone(),
                scheduled_at,
                payload,
                max_fee_lamports,
                max_account_creation_lamports,
                created_at: now,
            },
            next_state,
        }
    }

    fn choose_transition(
        &self,
        current: SessionState,
        rng: &mut ChaCha12Rng,
    ) -> Result<SessionState, CookerError> {
        let row: Vec<_> = self
            .persona
            .transitions
            .iter()
            .filter(|entry| entry.from == current && entry.weight > 0)
            .collect();
        let total: u64 = row.iter().map(|entry| u64::from(entry.weight)).sum();
        if total == 0 {
            return Err(CookerError::InvalidConfig(format!(
                "transition row for {current:?} has zero total weight"
            )));
        }
        let mut roll = rng.gen_range(0..total);
        for entry in row {
            let weight = u64::from(entry.weight);
            if roll < weight {
                return Ok(entry.to);
            }
            roll -= weight;
        }
        Err(CookerError::InvalidConfig(
            "weighted transition selection exhausted its row".to_owned(),
        ))
    }

    fn sample_dwell(&self, state: SessionState, rng: &mut ChaCha12Rng) -> Result<i64, CookerError> {
        let range = self.persona.state_durations.for_state(state);
        let sampled = rng.gen_range(range.min_seconds..=range.max_seconds);
        i64::try_from(sampled)
            .map_err(|_| CookerError::InvalidConfig("state duration exceeds i64".to_owned()))
    }

    fn choose_payload(
        &self,
        remaining_daily_budget: u64,
        rng: &mut ChaCha12Rng,
    ) -> Result<ActionPayload, CookerError> {
        let choices: Vec<_> = self
            .persona
            .action_weights
            .iter()
            .filter(|entry| entry.weight > 0 && self.catalog.supports(entry.kind))
            .collect();
        let total: u64 = choices.iter().map(|entry| u64::from(entry.weight)).sum();
        if total == 0 {
            return Ok(ActionPayload::Idle);
        }
        let mut roll = rng.gen_range(0..total);
        let mut selected = ActionKind::Idle;
        for entry in choices {
            let weight = u64::from(entry.weight);
            if roll < weight {
                selected = entry.kind;
                break;
            }
            roll -= weight;
        }
        if selected == ActionKind::Idle {
            return Ok(ActionPayload::Idle);
        }
        let amount = self.sample_amount(remaining_daily_budget, rng)?;
        if amount == 0 {
            return Ok(ActionPayload::Idle);
        }
        match selected {
            ActionKind::NativeTransfer => {
                let destination = choose(&self.catalog.native_destinations, rng)?;
                Ok(ActionPayload::NativeTransfer {
                    destination: destination.clone(),
                    lamports: amount,
                })
            }
            ActionKind::SplTransfer => {
                let route = choose(&self.catalog.spl_routes, rng)?;
                Ok(ActionPayload::SplTransfer {
                    mint: route.mint.clone(),
                    destination_owner: route.destination_owner.clone(),
                    amount,
                })
            }
            ActionKind::JupiterSwap => {
                let route = choose(&self.catalog.swap_routes, rng)?;
                Ok(ActionPayload::JupiterSwap {
                    input_mint: route.input_mint.clone(),
                    output_mint: route.output_mint.clone(),
                    amount,
                    max_slippage_bps: self.max_slippage_bps,
                })
            }
            ActionKind::StakeLifecycle => Ok(ActionPayload::StakeLifecycle {
                operation: StakeOperation::Enter,
                lamports: amount,
                stake_account: None,
            }),
            ActionKind::Idle => Ok(ActionPayload::Idle),
        }
    }

    fn sample_amount(
        &self,
        remaining_daily_budget: u64,
        rng: &mut ChaCha12Rng,
    ) -> Result<u64, CookerError> {
        let profile = &self.persona.amount;
        if remaining_daily_budget < profile.min_amount {
            return Ok(0);
        }
        let anchor = *choose(&profile.anchors, rng)?;
        let jitter = u64::try_from(
            u128::from(anchor) * u128::from(profile.jitter_bps) / u128::from(BASIS_POINTS),
        )
        .map_err(|_| CookerError::InvalidConfig("amount jitter overflow".to_owned()))?;
        let low = anchor.saturating_sub(jitter).max(profile.min_amount);
        let high = anchor
            .saturating_add(jitter)
            .min(profile.max_amount)
            .min(remaining_daily_budget);
        if high < low {
            return Ok(0);
        }
        let mut amount = rng.gen_range(low..=high);
        if rng.gen_range(0..BASIS_POINTS) < u64::from(profile.rounding_probability_bps) {
            amount = round_nearest(amount, profile.round_to);
        }
        Ok(amount
            .clamp(profile.min_amount, profile.max_amount)
            .min(remaining_daily_budget))
    }

    fn is_eligible_now(&self, agent: &AgentSnapshot, now: DateTime<Utc>) -> bool {
        let local = now + TimeDelta::minutes(i64::from(self.persona.utc_offset_minutes));
        let minute = u16::try_from(local.hour()).unwrap_or(0) * 60
            + u16::try_from(local.minute()).unwrap_or(0);
        self.persona.active_windows.iter().any(|window| {
            window.contains(minute)
                && self.participates(
                    agent,
                    participation_date(local.date_naive(), minute, *window),
                )
        })
    }

    fn participates(&self, agent: &AgentSnapshot, local_date: NaiveDate) -> bool {
        let mut hasher = blake3::Hasher::new();
        hasher.update(&self.global_seed);
        hasher.update(agent.id.0.as_bytes());
        hasher.update(&local_date.num_days_from_ce().to_le_bytes());
        hasher.update(b"daily-participation-v1");
        let digest = hasher.finalize();
        let bytes = digest.as_bytes();
        let sample = u16::from_le_bytes([bytes[0], bytes[1]]) % 10_000;
        sample < self.persona.daily_participation_bps
    }

    fn next_eligible_time(
        &self,
        agent: &AgentSnapshot,
        now: DateTime<Utc>,
    ) -> Result<DateTime<Utc>, CookerError> {
        let offset = TimeDelta::minutes(i64::from(self.persona.utc_offset_minutes));
        let local_now = now + offset;
        let first_date = local_now.date_naive();
        let mut best: Option<DateTime<Utc>> = None;
        for day_offset in 0..ELIGIBILITY_SEARCH_DAYS {
            let date = first_date
                .checked_add_days(Days::new(day_offset))
                .ok_or_else(|| CookerError::InvalidConfig("eligible date overflow".to_owned()))?;
            if !self.participates(agent, date) {
                continue;
            }
            for window in &self.persona.active_windows {
                let start = date
                    .and_hms_opt(0, 0, 0)
                    .and_then(|midnight| {
                        midnight
                            .checked_add_signed(TimeDelta::minutes(i64::from(window.start_minute)))
                    })
                    .ok_or_else(|| {
                        CookerError::InvalidConfig("window start overflow".to_owned())
                    })?;
                let jitter_limit = window_duration_seconds(*window)
                    .saturating_sub(1)
                    .min(u64::from(self.persona.session_start_jitter_seconds));
                let jitter = self.session_jitter(agent, date, window.start_minute, jitter_limit);
                let candidate = start
                    .checked_add_signed(TimeDelta::seconds(i64::try_from(jitter).map_err(
                        |_| CookerError::InvalidConfig("session jitter exceeds i64".to_owned()),
                    )?))
                    .ok_or_else(|| CookerError::InvalidConfig("window jitter overflow".to_owned()))?
                    .and_utc()
                    - offset;
                if candidate > now && best.is_none_or(|current| candidate < current) {
                    best = Some(candidate);
                }
            }
            if best.is_some() {
                break;
            }
        }
        best.ok_or_else(|| {
            CookerError::InvalidConfig(
                "no participating active window found within search horizon".to_owned(),
            )
        })
    }

    fn session_jitter(
        &self,
        agent: &AgentSnapshot,
        date: NaiveDate,
        window_start: u16,
        limit: u64,
    ) -> u64 {
        if limit == 0 {
            return 0;
        }
        let mut hasher = blake3::Hasher::new();
        hasher.update(&self.global_seed);
        hasher.update(agent.id.0.as_bytes());
        hasher.update(&date.num_days_from_ce().to_le_bytes());
        hasher.update(&window_start.to_le_bytes());
        hasher.update(b"session-jitter-v1");
        let bytes = hasher.finalize();
        let prefix: [u8; 8] = bytes.as_bytes()[..8].try_into().unwrap_or([0; 8]);
        u64::from_le_bytes(prefix) % (limit + 1)
    }
}

impl BehaviorModel for PersonaBehaviorModel {
    fn plan_next(
        &self,
        agent: &AgentSnapshot,
        now: DateTime<Utc>,
        rng: &mut ChaCha12Rng,
    ) -> Result<PlannedAction, CookerError> {
        self.plan_decision(agent, now, rng)
            .map(|decision| decision.action)
    }
}

fn choose<'a, T>(values: &'a [T], rng: &mut ChaCha12Rng) -> Result<&'a T, CookerError> {
    if values.is_empty() {
        return Err(CookerError::InvalidConfig(
            "cannot choose from an empty catalog".to_owned(),
        ));
    }
    Ok(&values[rng.gen_range(0..values.len())])
}

fn round_nearest(value: u64, quantum: u64) -> u64 {
    value
        .saturating_add(quantum / 2)
        .checked_div(quantum)
        .unwrap_or(0)
        .saturating_mul(quantum)
}

fn participation_date(date: NaiveDate, minute: u16, window: ActiveWindow) -> NaiveDate {
    if window.start_minute > window.end_minute && minute < window.end_minute {
        date.pred_opt().unwrap_or(date)
    } else {
        date
    }
}

fn window_duration_seconds(window: ActiveWindow) -> u64 {
    let minutes = if window.start_minute < window.end_minute {
        window.end_minute - window.start_minute
    } else {
        24 * 60 - window.start_minute + window.end_minute
    };
    u64::from(minutes) * 60
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use proptest::prelude::*;
    use rand_chacha::rand_core::SeedableRng;
    use uuid::Uuid;

    use super::*;
    use crate::{
        domain::{AgentId, RunId},
        persona::{PersonaPreset, StateTransition},
    };

    fn catalog() -> ActionCatalog {
        ActionCatalog {
            native_destinations: vec!["destination-a".to_owned()],
            spl_routes: vec![],
            swap_routes: vec![SwapRoute {
                input_mint: "mint-a".to_owned(),
                output_mint: "mint-b".to_owned(),
            }],
            allow_stake_enter: false,
        }
    }

    fn snapshot(state: SessionState) -> AgentSnapshot {
        AgentSnapshot {
            id: AgentId(Uuid::from_u128(11)),
            run_id: RunId(Uuid::from_u128(12)),
            next_sequence: 7,
            next_decision_at: Utc.with_ymd_and_hms(2026, 7, 17, 15, 0, 0).unwrap(),
            budget_date: chrono::NaiveDate::from_ymd_opt(2026, 7, 17).unwrap(),
            session_state: state,
            last_action_at: None,
            remaining_daily_budget: 1_000_000_000,
            model_version: "behavior-v1".to_owned(),
        }
    }

    #[test]
    fn identical_decision_rng_produces_identical_action() {
        let model = PersonaBehaviorModel::new(
            [4; 32],
            "behavior-v1",
            PersonaConfig::preset(PersonaPreset::Trader),
            catalog(),
            10_000,
            10_000_000,
            50,
        )
        .unwrap();
        let now = Utc.with_ymd_and_hms(2026, 7, 17, 15, 0, 0).unwrap();
        let mut left = ChaCha12Rng::from_seed([9; 32]);
        let mut right = ChaCha12Rng::from_seed([9; 32]);
        let left = model
            .plan_decision(&snapshot(SessionState::Active), now, &mut left)
            .unwrap();
        let right = model
            .plan_decision(&snapshot(SessionState::Active), now, &mut right)
            .unwrap();
        assert_eq!(left, right);
    }

    #[test]
    fn forced_transacting_transition_samples_a_bounded_amount() {
        let mut persona = PersonaConfig::preset(PersonaPreset::Trader);
        persona.daily_participation_bps = 10_000;
        persona
            .transitions
            .retain(|entry| entry.from != SessionState::Active);
        persona.transitions.push(StateTransition {
            from: SessionState::Active,
            to: SessionState::Transacting,
            weight: 1,
        });
        let min = persona.amount.min_amount;
        let max = persona.amount.max_amount;
        let model = PersonaBehaviorModel::new(
            [4; 32],
            "behavior-v1",
            persona,
            catalog(),
            10_000,
            10_000_000,
            50,
        )
        .unwrap();
        let now = Utc.with_ymd_and_hms(2026, 7, 17, 15, 0, 0).unwrap();
        for seed in 0_u8..100 {
            let mut rng = ChaCha12Rng::from_seed([seed; 32]);
            let decision = model
                .plan_decision(&snapshot(SessionState::Active), now, &mut rng)
                .unwrap();
            let amount = decision.action.payload.input_amount();
            if !matches!(decision.action.payload, ActionPayload::Idle) {
                assert!((min..=max).contains(&amount));
            }
        }
    }

    #[test]
    fn outside_active_window_schedules_idle_in_the_future() {
        let model = PersonaBehaviorModel::new(
            [4; 32],
            "behavior-v1",
            PersonaConfig::preset(PersonaPreset::Casual),
            catalog(),
            10_000,
            10_000_000,
            50,
        )
        .unwrap();
        let now = Utc.with_ymd_and_hms(2026, 7, 17, 8, 0, 0).unwrap();
        let mut rng = ChaCha12Rng::from_seed([1; 32]);
        let decision = model
            .plan_decision(&snapshot(SessionState::Dormant), now, &mut rng)
            .unwrap();
        assert_eq!(decision.next_state, SessionState::Dormant);
        assert!(matches!(decision.action.payload, ActionPayload::Idle));
        assert!(decision.action.scheduled_at > now);
    }

    #[test]
    fn amount_sampling_never_exceeds_remaining_budget() {
        let model = PersonaBehaviorModel::new(
            [4; 32],
            "behavior-v1",
            PersonaConfig::preset(PersonaPreset::Trader),
            catalog(),
            10_000,
            10_000_000,
            50,
        )
        .unwrap();
        let mut rng = ChaCha12Rng::from_seed([3; 32]);
        for budget in [999_999, 1_000_000, 1_500_000, 9_999_999] {
            assert!(model.sample_amount(budget, &mut rng).unwrap() <= budget);
        }
    }

    proptest! {
        #[test]
        fn amount_mixture_respects_profile_and_remaining_budget(
            seed in any::<[u8; 32]>(),
            budget in 0_u64..=2_000_000_000,
        ) {
            let persona = PersonaConfig::preset(PersonaPreset::Trader);
            let minimum = persona.amount.min_amount;
            let maximum = persona.amount.max_amount;
            let model = PersonaBehaviorModel::new(
                [4; 32],
                "behavior-v1",
                persona,
                catalog(),
                10_000,
                10_000_000,
                50,
            )
            .unwrap();
            let mut rng = ChaCha12Rng::from_seed(seed);
            let amount = model.sample_amount(budget, &mut rng).unwrap();

            prop_assert!(amount <= budget);
            prop_assert!(amount == 0 || (minimum..=maximum).contains(&amount));
            prop_assert!(budget >= minimum || amount == 0);
        }
    }
}
