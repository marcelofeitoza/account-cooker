//! Validated, integer-only configuration for semi-Markov behavior personas.

use serde::{Deserialize, Serialize};

use crate::{
    domain::{ActionKind, SessionState},
    error::CookerError,
};

const MINUTES_PER_DAY: u16 = 24 * 60;
const BASIS_POINTS: u16 = 10_000;
const MAX_DWELL_SECONDS: u64 = 7 * 24 * 60 * 60;

/// Named behavior presets shipped with the cooker.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersonaPreset {
    /// Infrequent activity around morning and evening windows.
    Casual,
    /// Frequent, bursty swap-oriented sessions.
    Trader,
    /// Sparse transfers and stateful holding periods.
    Saver,
    /// Broad action variety with a later active window.
    Explorer,
}

/// A local-time half-open activity interval, supporting midnight crossing.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ActiveWindow {
    /// Inclusive minute after local midnight, in `0..1440`.
    pub start_minute: u16,
    /// Exclusive minute after local midnight, in `0..1440`.
    pub end_minute: u16,
}

impl ActiveWindow {
    /// Construct and validate a window.
    pub fn new(start_minute: u16, end_minute: u16) -> Result<Self, CookerError> {
        let window = Self {
            start_minute,
            end_minute,
        };
        window.validate()?;
        Ok(window)
    }

    /// Whether a local minute falls in this interval.
    #[must_use]
    pub const fn contains(self, minute: u16) -> bool {
        if self.start_minute < self.end_minute {
            minute >= self.start_minute && minute < self.end_minute
        } else {
            minute >= self.start_minute || minute < self.end_minute
        }
    }

    fn validate(self) -> Result<(), CookerError> {
        if self.start_minute >= MINUTES_PER_DAY || self.end_minute >= MINUTES_PER_DAY {
            return Err(CookerError::InvalidConfig(
                "active-window minutes must be below 1440".to_owned(),
            ));
        }
        if self.start_minute == self.end_minute {
            return Err(CookerError::InvalidConfig(
                "active window cannot cover zero or twenty-four hours".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Inclusive dwell-time bounds for one state.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DurationRange {
    /// Shortest permitted dwell.
    pub min_seconds: u64,
    /// Longest permitted dwell.
    pub max_seconds: u64,
}

impl DurationRange {
    /// Construct and validate duration bounds.
    pub fn new(min_seconds: u64, max_seconds: u64) -> Result<Self, CookerError> {
        let range = Self {
            min_seconds,
            max_seconds,
        };
        range.validate()?;
        Ok(range)
    }

    fn validate(self) -> Result<(), CookerError> {
        if self.min_seconds == 0 || self.max_seconds < self.min_seconds {
            return Err(CookerError::InvalidConfig(
                "state duration must be positive and ordered".to_owned(),
            ));
        }
        if self.max_seconds > MAX_DWELL_SECONDS {
            return Err(CookerError::InvalidConfig(
                "state duration cannot exceed seven days".to_owned(),
            ));
        }
        Ok(())
    }
}

/// State-specific dwell distributions for the semi-Markov model.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StateDurations {
    /// Dormant-state dwell.
    pub dormant: DurationRange,
    /// Active-state dwell.
    pub active: DurationRange,
    /// Transacting-state dwell, which controls within-session bursts.
    pub transacting: DurationRange,
    /// Holding-state dwell.
    pub holding: DurationRange,
    /// Cool-down dwell.
    pub cooling_down: DurationRange,
}

impl StateDurations {
    /// Return bounds for a particular state.
    #[must_use]
    pub const fn for_state(&self, state: SessionState) -> DurationRange {
        match state {
            SessionState::Dormant => self.dormant,
            SessionState::Active => self.active,
            SessionState::Transacting => self.transacting,
            SessionState::Holding => self.holding,
            SessionState::CoolingDown => self.cooling_down,
        }
    }

    fn validate(&self) -> Result<(), CookerError> {
        self.dormant.validate()?;
        self.active.validate()?;
        self.transacting.validate()?;
        self.holding.validate()?;
        self.cooling_down.validate()
    }
}

/// One weighted state transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StateTransition {
    /// Source state.
    pub from: SessionState,
    /// Destination state.
    pub to: SessionState,
    /// Relative integer weight.
    pub weight: u32,
}

/// One weighted on-chain action family.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ActionWeight {
    /// Action family.
    pub kind: ActionKind,
    /// Relative integer weight.
    pub weight: u32,
}

/// Bounded human-style amount anchors, jitter, and optional rounding.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AmountProfile {
    /// Candidate central values in raw units.
    pub anchors: Vec<u64>,
    /// Hard lower bound after jitter and rounding.
    pub min_amount: u64,
    /// Hard upper bound after jitter and rounding.
    pub max_amount: u64,
    /// Maximum symmetric variation from an anchor, in basis points.
    pub jitter_bps: u16,
    /// Probability of rounding, in basis points.
    pub rounding_probability_bps: u16,
    /// Raw-unit quantum used when rounding.
    pub round_to: u64,
}

impl AmountProfile {
    fn validate(&self) -> Result<(), CookerError> {
        if self.anchors.is_empty() {
            return Err(CookerError::InvalidConfig(
                "amount profile requires at least one anchor".to_owned(),
            ));
        }
        if self.min_amount == 0 || self.max_amount < self.min_amount {
            return Err(CookerError::InvalidConfig(
                "amount bounds must be positive and ordered".to_owned(),
            ));
        }
        if self.jitter_bps > BASIS_POINTS
            || self.rounding_probability_bps > BASIS_POINTS
            || self.round_to == 0
        {
            return Err(CookerError::InvalidConfig(
                "amount jitter/rounding configuration is invalid".to_owned(),
            ));
        }
        if self
            .anchors
            .iter()
            .any(|anchor| *anchor < self.min_amount || *anchor > self.max_amount)
        {
            return Err(CookerError::InvalidConfig(
                "amount anchors must fall inside hard bounds".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Fully validated persona definition.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PersonaConfig {
    /// Stable operator-facing name.
    pub name: String,
    /// Fixed local offset from UTC, bounded to real-world time zones.
    pub utc_offset_minutes: i16,
    /// Non-overlapping local activity windows.
    pub active_windows: Vec<ActiveWindow>,
    /// Stable per-day probability of participating, in basis points.
    pub daily_participation_bps: u16,
    /// Per-agent delay from the beginning of an eligible activity window.
    pub session_start_jitter_seconds: u32,
    /// State-dependent dwell bounds.
    pub state_durations: StateDurations,
    /// Semi-Markov transition rows.
    pub transitions: Vec<StateTransition>,
    /// Actions selected only while entering or remaining in `Transacting`.
    pub action_weights: Vec<ActionWeight>,
    /// Bounded amount sampling configuration.
    pub amount: AmountProfile,
}

impl PersonaConfig {
    /// Return one built-in, validated persona.
    #[must_use]
    pub fn preset(preset: PersonaPreset) -> Self {
        let config = match preset {
            PersonaPreset::Casual => casual(),
            PersonaPreset::Trader => trader(),
            PersonaPreset::Saver => saver(),
            PersonaPreset::Explorer => explorer(),
        };
        debug_assert!(config.validate().is_ok());
        config
    }

    /// Validate windows, distributions, transition rows, and action weights.
    pub fn validate(&self) -> Result<(), CookerError> {
        if self.name.trim().is_empty() {
            return Err(CookerError::InvalidConfig(
                "persona name cannot be empty".to_owned(),
            ));
        }
        if !(-12 * 60..=14 * 60).contains(&self.utc_offset_minutes) {
            return Err(CookerError::InvalidConfig(
                "persona UTC offset must be between -12:00 and +14:00".to_owned(),
            ));
        }
        if self.active_windows.is_empty() {
            return Err(CookerError::InvalidConfig(
                "persona requires at least one active window".to_owned(),
            ));
        }
        let mut occupied = [false; MINUTES_PER_DAY as usize];
        for window in &self.active_windows {
            window.validate()?;
            for minute in 0..MINUTES_PER_DAY {
                if window.contains(minute) {
                    if occupied[usize::from(minute)] {
                        return Err(CookerError::InvalidConfig(
                            "persona active windows cannot overlap".to_owned(),
                        ));
                    }
                    occupied[usize::from(minute)] = true;
                }
            }
        }
        if self.daily_participation_bps == 0 || self.daily_participation_bps > BASIS_POINTS {
            return Err(CookerError::InvalidConfig(
                "daily participation must be in 1..=10000 basis points".to_owned(),
            ));
        }
        self.state_durations.validate()?;
        self.amount.validate()?;
        validate_transition_rows(&self.transitions)?;
        if self.action_weights.is_empty()
            || self.action_weights.iter().all(|entry| entry.weight == 0)
        {
            return Err(CookerError::InvalidConfig(
                "persona requires a positive action weight".to_owned(),
            ));
        }
        Ok(())
    }
}

fn validate_transition_rows(transitions: &[StateTransition]) -> Result<(), CookerError> {
    let states = [
        SessionState::Dormant,
        SessionState::Active,
        SessionState::Transacting,
        SessionState::Holding,
        SessionState::CoolingDown,
    ];
    for state in states {
        let row = transitions.iter().filter(|entry| entry.from == state);
        if row.clone().all(|entry| entry.weight == 0) {
            return Err(CookerError::InvalidConfig(format!(
                "transition row for {state:?} requires positive weight"
            )));
        }
    }
    Ok(())
}

fn duration(min_seconds: u64, max_seconds: u64) -> DurationRange {
    DurationRange {
        min_seconds,
        max_seconds,
    }
}

fn standard_transitions(transacting_repeat: u32, holding_repeat: u32) -> Vec<StateTransition> {
    use SessionState::{Active, CoolingDown, Dormant, Holding, Transacting};
    // Rows are read as (from, to, weight); every row's weights are relative within its `from`.
    [
        (Dormant, Active, 85),
        (Dormant, Dormant, 15),
        (Active, Transacting, 65),
        (Active, Holding, 10),
        (Active, CoolingDown, 15),
        (Active, Active, 10),
        (Transacting, Transacting, transacting_repeat),
        (Transacting, Holding, 15),
        (Transacting, CoolingDown, 45),
        (Transacting, Active, 10),
        (Holding, Holding, holding_repeat),
        (Holding, Active, 30),
        (Holding, CoolingDown, 20),
        (CoolingDown, Dormant, 55),
        (CoolingDown, Active, 40),
        (CoolingDown, CoolingDown, 5),
    ]
    .into_iter()
    .map(|(from, to, weight)| StateTransition { from, to, weight })
    .collect()
}

/// Build a relative action-weight table from `(kind, weight)` rows.
fn action_weights<const N: usize>(rows: [(ActionKind, u32); N]) -> Vec<ActionWeight> {
    rows.into_iter()
        .map(|(kind, weight)| ActionWeight { kind, weight })
        .collect()
}

fn common(
    name: &str,
    utc_offset_minutes: i16,
    active_windows: Vec<ActiveWindow>,
    participation: u16,
    transacting_repeat: u32,
    holding_repeat: u32,
) -> PersonaConfig {
    PersonaConfig {
        name: name.to_owned(),
        utc_offset_minutes,
        active_windows,
        daily_participation_bps: participation,
        session_start_jitter_seconds: 25 * 60,
        state_durations: StateDurations {
            dormant: duration(20 * 60, 3 * 60 * 60),
            active: duration(3 * 60, 35 * 60),
            transacting: duration(45, 12 * 60),
            holding: duration(20 * 60, 4 * 60 * 60),
            cooling_down: duration(15 * 60, 2 * 60 * 60),
        },
        transitions: standard_transitions(transacting_repeat, holding_repeat),
        action_weights: Vec::new(),
        amount: AmountProfile {
            anchors: vec![5_000_000, 10_000_000, 25_000_000, 50_000_000],
            min_amount: 1_000_000,
            max_amount: 100_000_000,
            jitter_bps: 2_200,
            rounding_probability_bps: 3_500,
            round_to: 100_000,
        },
    }
}

fn window(start_hour: u16, start_minute: u16, end_hour: u16, end_minute: u16) -> ActiveWindow {
    ActiveWindow {
        start_minute: start_hour * 60 + start_minute,
        end_minute: end_hour * 60 + end_minute,
    }
}

fn casual() -> PersonaConfig {
    let mut config = common(
        "casual",
        -3 * 60,
        vec![window(7, 30, 9, 15), window(18, 0, 23, 0)],
        6_200,
        25,
        50,
    );
    config.action_weights = action_weights([
        (ActionKind::NativeTransfer, 35),
        (ActionKind::JupiterSwap, 55),
        (ActionKind::Idle, 10),
    ]);
    config
}

fn trader() -> PersonaConfig {
    let mut config = common("trader", -3 * 60, vec![window(8, 0, 23, 30)], 9_000, 65, 20);
    config.state_durations.transacting = duration(20, 5 * 60);
    config.amount.jitter_bps = 3_000;
    config.action_weights = action_weights([
        (ActionKind::NativeTransfer, 10),
        (ActionKind::JupiterSwap, 85),
        (ActionKind::Idle, 5),
    ]);
    config
}

fn saver() -> PersonaConfig {
    let mut config = common(
        "saver",
        -3 * 60,
        vec![window(6, 30, 8, 15), window(18, 30, 21, 30)],
        4_500,
        15,
        75,
    );
    config.state_durations.holding = duration(60 * 60, 6 * 60 * 60);
    config.action_weights = action_weights([
        (ActionKind::NativeTransfer, 60),
        (ActionKind::JupiterSwap, 25),
        (ActionKind::Idle, 15),
    ]);
    config
}

fn explorer() -> PersonaConfig {
    let mut config = common(
        "explorer",
        -3 * 60,
        vec![window(11, 0, 2, 0)],
        7_500,
        40,
        35,
    );
    config.action_weights = action_weights([
        (ActionKind::NativeTransfer, 30),
        (ActionKind::SplTransfer, 20),
        (ActionKind::JupiterSwap, 45),
        (ActionKind::Idle, 5),
    ]);
    config
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_is_valid() {
        for preset in [
            PersonaPreset::Casual,
            PersonaPreset::Trader,
            PersonaPreset::Saver,
            PersonaPreset::Explorer,
        ] {
            PersonaConfig::preset(preset).validate().unwrap();
        }
    }

    #[test]
    fn crossing_midnight_window_contains_both_sides() {
        let window = ActiveWindow::new(22 * 60, 2 * 60).unwrap();
        assert!(window.contains(23 * 60));
        assert!(window.contains(60));
        assert!(!window.contains(12 * 60));
    }

    #[test]
    fn overlapping_windows_are_rejected() {
        let mut config = PersonaConfig::preset(PersonaPreset::Casual);
        config.active_windows = vec![window(8, 0, 12, 0), window(11, 0, 13, 0)];
        assert!(config.validate().is_err());
    }

    #[test]
    fn every_state_requires_a_transition_row() {
        let mut config = PersonaConfig::preset(PersonaPreset::Casual);
        config
            .transitions
            .retain(|entry| entry.from != SessionState::Holding);
        assert!(config.validate().is_err());
    }
}
