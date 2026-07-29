//! Transparent pairwise linkage attacks over extracted observations.

use std::collections::{BTreeMap, BTreeSet};

use cooker_core::AgentId;
use serde::{Deserialize, Serialize};

use crate::{
    features::{AgentFeatures, FeatureSet},
    metrics::count_as_f64,
};

/// Independently reported observable feature family.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureFamily {
    /// Time-of-day and cadence similarity.
    Timing,
    /// Amount magnitude, collision, and roundness similarity.
    Amount,
    /// Action bigram similarity.
    Sequence,
    /// Destination-set overlap.
    Destination,
    /// Near-simultaneous event ratio.
    Synchrony,
    /// Common-funder overlap.
    Funding,
    /// Common funding-round overlap, which reads the payer and the batching round together.
    FundingRound,
    /// Scheme-aware co-funding, which weights a shared payer-and-round batch by how few
    /// accounts were in it.
    FundingBatch,
    /// Protocol-route overlap.
    Route,
    /// Mean balance-rank proximity.
    BalanceRank,
}

impl FeatureFamily {
    /// Every family, in the fixed order every per-family report uses.
    pub const ALL: [Self; 10] = [
        Self::Timing,
        Self::Amount,
        Self::Sequence,
        Self::Destination,
        Self::Synchrony,
        Self::Funding,
        Self::FundingRound,
        Self::FundingBatch,
        Self::Route,
        Self::BalanceRank,
    ];
}

/// Fixed composite weights selected without evaluation labels.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AttackWeights {
    /// Timing weight.
    pub timing: f64,
    /// Amount weight.
    pub amount: f64,
    /// Sequence weight.
    pub sequence: f64,
    /// Destination weight.
    pub destination: f64,
    /// Synchrony weight.
    pub synchrony: f64,
    /// Common-funder weight.
    pub funding: f64,
    /// Funding-round weight. The fixed composite leaves this at zero so composite results
    /// stay comparable with the pre-mitigation baseline; the family is always reported on
    /// its own at full strength.
    pub funding_round: f64,
    /// Scheme-aware co-funding weight. Held at zero for the same comparability reason as
    /// `funding_round`, and reported on its own at full strength.
    pub funding_batch: f64,
    /// Route weight.
    pub route: f64,
    /// Balance-rank weight.
    pub balance_rank: f64,
}

impl Default for AttackWeights {
    fn default() -> Self {
        Self {
            timing: 0.18,
            amount: 0.15,
            sequence: 0.15,
            destination: 0.10,
            synchrony: 0.14,
            funding: 0.14,
            funding_round: 0.0,
            funding_batch: 0.0,
            route: 0.08,
            balance_rank: 0.06,
        }
    }
}

impl AttackWeights {
    /// Return a copy with one feature removed and remaining weights normalized.
    #[must_use]
    pub fn without(self, family: FeatureFamily) -> Self {
        let mut output = self;
        match family {
            FeatureFamily::Timing => output.timing = 0.0,
            FeatureFamily::Amount => output.amount = 0.0,
            FeatureFamily::Sequence => output.sequence = 0.0,
            FeatureFamily::Destination => output.destination = 0.0,
            FeatureFamily::Synchrony => output.synchrony = 0.0,
            FeatureFamily::Funding => output.funding = 0.0,
            FeatureFamily::FundingRound => output.funding_round = 0.0,
            FeatureFamily::FundingBatch => output.funding_batch = 0.0,
            FeatureFamily::Route => output.route = 0.0,
            FeatureFamily::BalanceRank => output.balance_rank = 0.0,
        }
        output.normalized()
    }

    /// Clamp negative weights and normalize their sum to one.
    #[must_use]
    pub fn normalized(mut self) -> Self {
        self.timing = self.timing.max(0.0);
        self.amount = self.amount.max(0.0);
        self.sequence = self.sequence.max(0.0);
        self.destination = self.destination.max(0.0);
        self.synchrony = self.synchrony.max(0.0);
        self.funding = self.funding.max(0.0);
        self.funding_round = self.funding_round.max(0.0);
        self.funding_batch = self.funding_batch.max(0.0);
        self.route = self.route.max(0.0);
        self.balance_rank = self.balance_rank.max(0.0);
        let sum = self.timing
            + self.amount
            + self.sequence
            + self.destination
            + self.synchrony
            + self.funding
            + self.funding_round
            + self.funding_batch
            + self.route
            + self.balance_rank;
        if sum > 0.0 {
            self.timing /= sum;
            self.amount /= sum;
            self.sequence /= sum;
            self.destination /= sum;
            self.synchrony /= sum;
            self.funding /= sum;
            self.funding_round /= sum;
            self.funding_batch /= sum;
            self.route /= sum;
            self.balance_rank /= sum;
        }
        self
    }
}

/// Pairwise linkage scores produced without controller labels.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PairScore {
    /// First stable agent identifier.
    pub left: AgentId,
    /// Second stable agent identifier.
    pub right: AgentId,
    /// Timing attack score in `[0, 1]`.
    pub timing: f64,
    /// Amount attack score in `[0, 1]`.
    pub amount: f64,
    /// Sequence attack score in `[0, 1]`.
    pub sequence: f64,
    /// Destination attack score in `[0, 1]`.
    pub destination: f64,
    /// Synchrony attack score in `[0, 1]`.
    pub synchrony: f64,
    /// Funding-graph attack score in `[0, 1]`.
    pub funding: f64,
    /// Funding-round attack score in `[0, 1]`.
    pub funding_round: f64,
    /// Scheme-aware co-funding attack score in `[0, 1]`.
    pub funding_batch: f64,
    /// Route attack score in `[0, 1]`.
    pub route: f64,
    /// Balance-rank attack score in `[0, 1]`.
    pub balance_rank: f64,
    /// Fixed weighted composite score in `[0, 1]`.
    pub composite: f64,
}

/// Score every unordered pair using fixed, label-independent weights.
#[must_use]
pub fn score_pairs(features: &FeatureSet, weights: AttackWeights) -> Vec<PairScore> {
    let weights = weights.normalized();
    let agents: Vec<_> = features.agents.iter().collect();
    let mut output = Vec::with_capacity(agents.len().saturating_mul(agents.len()) / 2);
    for left_index in 0..agents.len() {
        for right_index in (left_index + 1)..agents.len() {
            let (left_id, left) = agents[left_index];
            let (right_id, right) = agents[right_index];
            let timing = timing_score(left, right);
            let amount = amount_score(left, right);
            let sequence = cosine(&left.bigrams, &right.bigrams);
            let destination = jaccard(&left.destinations, &right.destinations);
            let synchrony = synchrony_score(
                &left.event_times,
                &right.event_times,
                features.synchrony_window_seconds,
            );
            let funding = jaccard(&left.funders, &right.funders);
            let funding_round = jaccard(&left.funding_rounds, &right.funding_rounds);
            let funding_batch = rarity_weighted_overlap(
                &left.funding_rounds,
                &right.funding_rounds,
                &features.funding_cell_agents,
            );
            let route = jaccard(&left.routes, &right.routes);
            let balance_rank = balance_rank_score(left.mean_balance_rank, right.mean_balance_rank);
            let composite = timing * weights.timing
                + amount * weights.amount
                + sequence * weights.sequence
                + destination * weights.destination
                + synchrony * weights.synchrony
                + funding * weights.funding
                + funding_round * weights.funding_round
                + funding_batch * weights.funding_batch
                + route * weights.route
                + balance_rank * weights.balance_rank;
            output.push(PairScore {
                left: *left_id,
                right: *right_id,
                timing,
                amount,
                sequence,
                destination,
                synchrony,
                funding,
                funding_round,
                funding_batch,
                route,
                balance_rank,
                composite: composite.clamp(0.0, 1.0),
            });
        }
    }
    output
}

fn timing_score(left: &AgentFeatures, right: &AgentFeatures) -> f64 {
    0.7 * cosine(&left.hours, &right.hours)
        + 0.3 * cadence_similarity(&left.intervals, &right.intervals)
}

fn amount_score(left: &AgentFeatures, right: &AgentFeatures) -> f64 {
    0.65 * cosine(&left.amount_histogram, &right.amount_histogram)
        + 0.20 * jaccard(&left.exact_amounts, &right.exact_amounts)
        + 0.15 * (1.0 - (left.round_amount_ratio - right.round_amount_ratio).abs())
}

fn cadence_similarity(left: &[f64], right: &[f64]) -> f64 {
    let Some((left_mean, left_cv)) = mean_cv(left) else {
        return 0.0;
    };
    let Some((right_mean, right_cv)) = mean_cv(right) else {
        return 0.0;
    };
    let mean_scale = left_mean.max(right_mean).max(1.0);
    let cv_scale = left_cv.max(right_cv).max(1.0);
    let mean_similarity = 1.0 - (left_mean - right_mean).abs() / mean_scale;
    let cv_similarity = 1.0 - (left_cv - right_cv).abs() / cv_scale;
    (0.5 * mean_similarity + 0.5 * cv_similarity).clamp(0.0, 1.0)
}

fn mean_cv(values: &[f64]) -> Option<(f64, f64)> {
    if values.is_empty() {
        return None;
    }
    let mean = values.iter().sum::<f64>() / count_as_f64(values.len());
    let variance = values
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / count_as_f64(values.len());
    Some((mean, variance.sqrt() / mean.max(1.0)))
}

fn cosine(left: &[f64], right: &[f64]) -> f64 {
    if left.len() != right.len() || left.is_empty() {
        return 0.0;
    }
    let dot = left
        .iter()
        .zip(right)
        .map(|(left, right)| left * right)
        .sum::<f64>();
    let left_norm = left.iter().map(|value| value * value).sum::<f64>().sqrt();
    let right_norm = right.iter().map(|value| value * value).sum::<f64>().sqrt();
    if left_norm == 0.0 || right_norm == 0.0 {
        0.0
    } else {
        (dot / (left_norm * right_norm)).clamp(0.0, 1.0)
    }
}

/// Score co-funding evidence the way an adversary who knows the pooling scheme would.
///
/// Plain overlap treats every shared payer-and-round batch alike. An adversary who knows
/// that a pool deals accounts into batches gains far more from a batch of two than from a
/// batch of twenty, because a small batch nearly names its members. Each cell therefore
/// carries weight `1 / ln(1 + members)`, and the score is the cosine of the two weighted
/// indicator vectors, which stays in `[0, 1]` and reaches one only for identical cell sets.
///
/// The member count is read from the observations themselves, so the attacker needs no
/// operator labels and no knowledge the chain does not already publish.
fn rarity_weighted_overlap(
    left: &BTreeSet<String>,
    right: &BTreeSet<String>,
    cell_agents: &BTreeMap<String, usize>,
) -> f64 {
    let weight = |cell: &String| {
        let members = cell_agents.get(cell).copied().unwrap_or(1).max(1);
        let scale = count_as_f64(members) + 1.0;
        1.0 / scale.ln().max(f64::MIN_POSITIVE)
    };
    let squared_norm = |set: &BTreeSet<String>| set.iter().map(|cell| weight(cell).powi(2)).sum();
    let left_norm: f64 = squared_norm(left);
    let right_norm: f64 = squared_norm(right);
    if left_norm <= 0.0 || right_norm <= 0.0 {
        return 0.0;
    }
    let shared: f64 = left
        .intersection(right)
        .map(|cell| weight(cell).powi(2))
        .sum();
    (shared / (left_norm * right_norm).sqrt()).clamp(0.0, 1.0)
}

fn jaccard<T: Ord>(left: &BTreeSet<T>, right: &BTreeSet<T>) -> f64 {
    let union = left.union(right).count();
    if union == 0 {
        0.0
    } else {
        count_as_f64(left.intersection(right).count()) / count_as_f64(union)
    }
}

fn synchrony_score(
    left: &[chrono::DateTime<chrono::Utc>],
    right: &[chrono::DateTime<chrono::Utc>],
    window_seconds: i64,
) -> f64 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let mut left_index = 0_usize;
    let mut right_index = 0_usize;
    let mut matches = 0_usize;
    while left_index < left.len() && right_index < right.len() {
        let difference = (right[right_index] - left[left_index]).num_seconds();
        if difference.abs() <= window_seconds {
            matches += 1;
            left_index += 1;
            right_index += 1;
        } else if difference < 0 {
            right_index += 1;
        } else {
            left_index += 1;
        }
    }
    count_as_f64(matches) / count_as_f64(left.len().min(right.len()))
}

fn balance_rank_score(left: Option<f64>, right: Option<f64>) -> f64 {
    match (left, right) {
        (Some(left), Some(right)) => {
            let scale = left.abs().max(right.abs()).max(1.0);
            (1.0 - (left - right).abs() / scale).clamp(0.0, 1.0)
        }
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ablation_removes_only_selected_weight() {
        let weights = AttackWeights::default().without(FeatureFamily::Funding);
        assert!(weights.funding.abs() < f64::EPSILON);
        let sum = weights.timing
            + weights.amount
            + weights.sequence
            + weights.destination
            + weights.synchrony
            + weights.funding
            + weights.funding_round
            + weights.funding_batch
            + weights.route
            + weights.balance_rank;
        assert!((sum - 1.0).abs() < 1e-12);
        assert!(weights.timing > 0.0);
    }

    fn cells(values: &[&str]) -> BTreeSet<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn rarity_weighting_prefers_small_shared_batches() {
        let counts = BTreeMap::from([("tiny".to_owned(), 2), ("wide".to_owned(), 200)]);
        let tiny = rarity_weighted_overlap(
            &cells(&["tiny", "solo-left"]),
            &cells(&["tiny", "solo-right"]),
            &counts,
        );
        let wide = rarity_weighted_overlap(
            &cells(&["wide", "solo-left"]),
            &cells(&["wide", "solo-right"]),
            &counts,
        );
        assert!(
            tiny > wide,
            "a shared two-account batch must outrank a shared two-hundred-account batch: {tiny} vs {wide}"
        );
        assert!((0.0..=1.0).contains(&tiny) && (0.0..=1.0).contains(&wide));
    }

    #[test]
    fn rarity_weighting_is_bounded_and_symmetric() {
        let counts = BTreeMap::from([("a".to_owned(), 3), ("b".to_owned(), 9)]);
        let left = cells(&["a", "b"]);
        let right = cells(&["a", "b"]);
        assert!((rarity_weighted_overlap(&left, &right, &counts) - 1.0).abs() < 1e-12);
        let other = cells(&["b"]);
        assert!(
            (rarity_weighted_overlap(&left, &other, &counts)
                - rarity_weighted_overlap(&other, &left, &counts))
            .abs()
                < 1e-12
        );
        assert!(rarity_weighted_overlap(&left, &cells(&[]), &counts).abs() < f64::EPSILON);
    }
}
