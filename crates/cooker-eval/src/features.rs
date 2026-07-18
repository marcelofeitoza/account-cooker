//! Deterministic per-agent feature extraction without controller labels.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Timelike, Utc};
use cooker_core::{ActionKind, AgentId, TraceEvent};
use serde::{Deserialize, Serialize};

const HOURS_PER_DAY: usize = 24;
const ACTION_KINDS: usize = 5;
const BIGRAMS: usize = ACTION_KINDS * ACTION_KINDS;

/// Feature extraction settings fixed before labels are supplied.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FeatureConfig {
    /// Maximum event-time difference counted as synchronous.
    pub synchrony_window_seconds: i64,
    /// Number of base-10 amount-magnitude buckets, from 1 to 19.
    pub amount_bins: usize,
}

impl Default for FeatureConfig {
    fn default() -> Self {
        Self {
            synchrony_window_seconds: 90,
            amount_bins: 12,
        }
    }
}

impl FeatureConfig {
    fn validated(self) -> Self {
        Self {
            synchrony_window_seconds: self.synchrony_window_seconds.max(0),
            amount_bins: self.amount_bins.clamp(1, 19),
        }
    }
}

/// Features grouped by pseudonymous agent.
#[derive(Clone, Debug, Default)]
pub struct FeatureSet {
    pub(crate) agents: BTreeMap<AgentId, AgentFeatures>,
    pub(crate) synchrony_window_seconds: i64,
}

impl FeatureSet {
    /// Number of agents with extracted observations.
    #[must_use]
    pub fn len(&self) -> usize {
        self.agents.len()
    }

    /// Whether no agents were observed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.agents.is_empty()
    }

    /// Stable ordered agent identifiers.
    pub fn agent_ids(&self) -> impl Iterator<Item = AgentId> + '_ {
        self.agents.keys().copied()
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct AgentFeatures {
    pub event_times: Vec<DateTime<Utc>>,
    pub hours: [f64; HOURS_PER_DAY],
    pub intervals: Vec<f64>,
    pub amount_histogram: Vec<f64>,
    pub exact_amounts: BTreeSet<u64>,
    pub round_amount_ratio: f64,
    pub bigrams: [f64; BIGRAMS],
    pub destinations: BTreeSet<String>,
    pub funders: BTreeSet<String>,
    pub routes: BTreeSet<String>,
    pub mean_balance_rank: Option<f64>,
}

/// Extract stable features from observations only.
#[must_use]
pub fn extract_features(events: &[TraceEvent], config: FeatureConfig) -> FeatureSet {
    let config = config.validated();
    let mut grouped: BTreeMap<AgentId, Vec<&TraceEvent>> = BTreeMap::new();
    for event in events {
        grouped.entry(event.agent_id).or_default().push(event);
    }

    let mut agents = BTreeMap::new();
    for (agent_id, mut agent_events) in grouped {
        agent_events.sort_by(|left, right| {
            event_time(left)
                .cmp(&event_time(right))
                .then(left.sequence.cmp(&right.sequence))
        });
        agents.insert(agent_id, extract_agent(&agent_events, config.amount_bins));
    }

    FeatureSet {
        agents,
        synchrony_window_seconds: config.synchrony_window_seconds,
    }
}

fn extract_agent(events: &[&TraceEvent], amount_bins: usize) -> AgentFeatures {
    let mut features = AgentFeatures {
        amount_histogram: vec![0.0; amount_bins],
        ..AgentFeatures::default()
    };
    let mut round_amounts = 0_usize;
    let mut balance_rank_total = 0.0;
    let mut balance_rank_count = 0_usize;
    let mut previous_kind = None;

    for event in events {
        let timestamp = event_time(event);
        features.event_times.push(timestamp);
        features.hours[timestamp.hour() as usize] += 1.0;

        let amount_bin = amount_bin(event.amount, amount_bins);
        features.amount_histogram[amount_bin] += 1.0;
        features.exact_amounts.insert(event.amount);
        if event.amount > 0 && event.amount.is_multiple_of(1_000) {
            round_amounts += 1;
        }
        if let Some(destination) = &event.destination {
            features.destinations.insert(destination.clone());
        }
        if let Some(funder) = event.attributes.get("funder") {
            features.funders.insert(funder.clone());
        }
        if let Some(route) = event.attributes.get("route") {
            features.routes.insert(route.clone());
        }
        if let Some(rank) = event
            .attributes
            .get("balance_rank")
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| value.is_finite())
        {
            balance_rank_total += rank;
            balance_rank_count += 1;
        }
        if let Some(previous) = previous_kind {
            features.bigrams
                [kind_index(previous) * ACTION_KINDS + kind_index(event.action_kind)] += 1.0;
        }
        previous_kind = Some(event.action_kind);
    }

    features.intervals = features
        .event_times
        .windows(2)
        .map(|window| {
            i32::try_from((window[1] - window[0]).num_seconds().max(0))
                .map_or(f64::from(i32::MAX), f64::from)
        })
        .collect();
    let event_count = events.len();
    if event_count > 0 {
        features.round_amount_ratio = count_as_f64(round_amounts) / count_as_f64(event_count);
    }
    if balance_rank_count > 0 {
        features.mean_balance_rank = Some(balance_rank_total / count_as_f64(balance_rank_count));
    }
    normalize(&mut features.hours);
    normalize_slice(&mut features.amount_histogram);
    normalize(&mut features.bigrams);
    features
}

fn event_time(event: &TraceEvent) -> DateTime<Utc> {
    event.observed_at.unwrap_or(event.scheduled_at)
}

fn amount_bin(amount: u64, bins: usize) -> usize {
    if amount == 0 {
        0
    } else {
        usize::try_from(amount.ilog10())
            .unwrap_or(usize::MAX)
            .min(bins - 1)
    }
}

fn kind_index(kind: ActionKind) -> usize {
    match kind {
        ActionKind::NativeTransfer => 0,
        ActionKind::SplTransfer => 1,
        ActionKind::JupiterSwap => 2,
        ActionKind::StakeLifecycle => 3,
        ActionKind::Idle => 4,
    }
}

fn normalize<const N: usize>(values: &mut [f64; N]) {
    normalize_slice(values);
}

fn normalize_slice(values: &mut [f64]) {
    let sum: f64 = values.iter().sum();
    if sum > 0.0 {
        for value in values {
            *value /= sum;
        }
    }
}

fn count_as_f64(value: usize) -> f64 {
    u32::try_from(value).map_or(f64::from(u32::MAX), f64::from)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::TimeZone;
    use cooker_core::{ActionId, RunId, TraceOutcome};
    use uuid::Uuid;

    use super::*;

    fn event(agent: AgentId, sequence: u64, hour: u32, amount: u64) -> Result<TraceEvent, String> {
        let timestamp = Utc
            .with_ymd_and_hms(2026, 7, 16, hour, 0, 0)
            .single()
            .ok_or_else(|| "invalid fixture timestamp".to_owned())?;
        Ok(TraceEvent {
            schema_version: TraceEvent::SCHEMA_VERSION,
            run_id: RunId(Uuid::nil()),
            agent_id: agent,
            action_id: ActionId::derive(RunId(Uuid::nil()), agent, sequence, "test"),
            sequence,
            action_kind: ActionKind::NativeTransfer,
            scheduled_at: timestamp,
            observed_at: Some(timestamp),
            amount,
            destination: Some("destination".to_owned()),
            signature: None,
            outcome: TraceOutcome::Confirmed,
            attributes: BTreeMap::new(),
        })
    }

    #[test]
    fn extraction_is_stable_under_input_order() -> Result<(), String> {
        let agent = AgentId(Uuid::from_u128(1));
        let left = vec![event(agent, 2, 12, 9_000)?, event(agent, 1, 8, 10)?];
        let right = vec![left[1].clone(), left[0].clone()];
        let left_features = extract_features(&left, FeatureConfig::default());
        let right_features = extract_features(&right, FeatureConfig::default());
        assert!(
            left_features.agents[&agent]
                .hours
                .iter()
                .zip(right_features.agents[&agent].hours)
                .all(|(left, right)| (*left - right).abs() < f64::EPSILON)
        );
        assert_eq!(
            left_features.agents[&agent].intervals,
            right_features.agents[&agent].intervals
        );
        Ok(())
    }
}
