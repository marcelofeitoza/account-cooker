//! Binary-linkage and clustering metrics with fixed evaluation thresholds.

use std::collections::{BTreeMap, BTreeSet};

use cooker_core::AgentId;
use serde::{Deserialize, Serialize};

use crate::{GroundTruth, PairScore};

/// Pairwise binary-classification metrics.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BinaryMetrics {
    /// Fixed threshold selected before held-out labels are loaded.
    pub threshold: f64,
    /// True positive pair count.
    pub true_positives: usize,
    /// False positive pair count.
    pub false_positives: usize,
    /// True negative pair count.
    pub true_negatives: usize,
    /// False negative pair count.
    pub false_negatives: usize,
    /// Positive predictive value.
    pub precision: f64,
    /// True positive rate.
    pub recall: f64,
    /// Harmonic mean of precision and recall.
    pub f1: f64,
    /// Receiver operating characteristic area under curve.
    pub roc_auc: f64,
    /// Fraction of true links among the highest-scored K pairs.
    pub precision_at_k: f64,
    /// K used for precision-at-K.
    pub k: usize,
}

/// Agreement between threshold-connected components and controller labels.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClusteringMetrics {
    /// Adjusted Rand index.
    pub adjusted_rand: f64,
    /// Normalized mutual information.
    pub normalized_mutual_information: f64,
    /// Number of predicted connected components.
    pub predicted_clusters: usize,
    /// Number of true controller groups.
    pub true_clusters: usize,
}

/// Complete metrics for one pair-score family.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EvaluationMetrics {
    /// Pairwise classification results.
    pub binary: BinaryMetrics,
    /// Threshold clustering results.
    pub clustering: ClusteringMetrics,
}

/// Evaluate precomputed pair scores against separately supplied labels.
///
/// # Errors
///
/// Returns an error for an invalid threshold or labels that do not exactly cover scored agents.
pub fn evaluate_scores(
    scores: &[PairScore],
    truth: &GroundTruth,
    threshold: f64,
) -> Result<EvaluationMetrics, String> {
    if !(0.0..=1.0).contains(&threshold) {
        return Err("threshold must be between zero and one".to_owned());
    }
    let agents: BTreeSet<_> = scores
        .iter()
        .flat_map(|pair| [pair.left, pair.right])
        .collect();
    truth.validate_agents(&agents)?;

    let mut labelled = Vec::with_capacity(scores.len());
    for pair in scores {
        let positive = truth
            .same_controller(pair.left, pair.right)
            .ok_or_else(|| "pair is missing a controller label".to_owned())?;
        labelled.push((pair.composite, positive, pair.left, pair.right));
    }

    let binary = binary_metrics(&labelled, threshold);
    let predicted = connected_components(&agents, &labelled, threshold);
    let clustering = clustering_metrics(&agents, &predicted, truth);
    Ok(EvaluationMetrics { binary, clustering })
}

fn binary_metrics(labelled: &[(f64, bool, AgentId, AgentId)], threshold: f64) -> BinaryMetrics {
    let mut true_positives = 0;
    let mut false_positives = 0;
    let mut true_negatives = 0;
    let mut false_negatives = 0;
    for (score, positive, _, _) in labelled {
        match (*score >= threshold, positive) {
            (true, true) => true_positives += 1,
            (true, false) => false_positives += 1,
            (false, false) => true_negatives += 1,
            (false, true) => false_negatives += 1,
        }
    }
    let precision = ratio(true_positives, true_positives + false_positives);
    let recall = ratio(true_positives, true_positives + false_negatives);
    let f1 = if precision + recall > 0.0 {
        2.0 * precision * recall / (precision + recall)
    } else {
        0.0
    };
    let positive_count = labelled.iter().filter(|(_, label, _, _)| *label).count();
    let k = positive_count.min(labelled.len());
    let mut ranked = labelled.to_vec();
    ranked.sort_by(|left, right| right.0.total_cmp(&left.0));
    let precision_at_k = if k == 0 {
        0.0
    } else {
        count_as_f64(ranked.iter().take(k).filter(|entry| entry.1).count()) / count_as_f64(k)
    };
    BinaryMetrics {
        threshold,
        true_positives,
        false_positives,
        true_negatives,
        false_negatives,
        precision,
        recall,
        f1,
        roc_auc: roc_auc(labelled),
        precision_at_k,
        k,
    }
}

fn roc_auc(labelled: &[(f64, bool, AgentId, AgentId)]) -> f64 {
    let positives: Vec<_> = labelled
        .iter()
        .filter(|entry| entry.1)
        .map(|entry| entry.0)
        .collect();
    let negatives: Vec<_> = labelled
        .iter()
        .filter(|entry| !entry.1)
        .map(|entry| entry.0)
        .collect();
    if positives.is_empty() || negatives.is_empty() {
        return 0.5;
    }
    let mut wins = 0.0;
    for positive in &positives {
        for negative in &negatives {
            wins += if positive > negative {
                1.0
            } else if (positive - negative).abs() <= f64::EPSILON {
                0.5
            } else {
                0.0
            };
        }
    }
    wins / count_as_f64(positives.len().saturating_mul(negatives.len()))
}

fn connected_components(
    agents: &BTreeSet<AgentId>,
    labelled: &[(f64, bool, AgentId, AgentId)],
    threshold: f64,
) -> BTreeMap<AgentId, usize> {
    let ordered: Vec<_> = agents.iter().copied().collect();
    let indices: BTreeMap<_, _> = ordered
        .iter()
        .enumerate()
        .map(|(index, agent)| (*agent, index))
        .collect();
    let mut union = UnionFind::new(ordered.len());
    for (score, _, left, right) in labelled {
        if *score >= threshold {
            union.join(indices[left], indices[right]);
        }
    }
    ordered
        .into_iter()
        .enumerate()
        .map(|(index, agent)| (agent, union.root(index)))
        .collect()
}

fn clustering_metrics(
    agents: &BTreeSet<AgentId>,
    predicted: &BTreeMap<AgentId, usize>,
    truth: &GroundTruth,
) -> ClusteringMetrics {
    let mut true_ids = BTreeMap::<String, usize>::new();
    let mut next_true_id = 0_usize;
    let assignments: Vec<_> = agents
        .iter()
        .map(|agent| {
            let label = &truth.controllers[agent];
            let true_id = *true_ids.entry(label.clone()).or_insert_with(|| {
                let assigned = next_true_id;
                next_true_id += 1;
                assigned
            });
            (true_id, predicted[agent])
        })
        .collect();
    let predicted_clusters = assignments
        .iter()
        .map(|assignment| assignment.1)
        .collect::<BTreeSet<_>>()
        .len();
    ClusteringMetrics {
        adjusted_rand: adjusted_rand(&assignments),
        normalized_mutual_information: normalized_mutual_information(&assignments),
        predicted_clusters,
        true_clusters: true_ids.len(),
    }
}

fn adjusted_rand(assignments: &[(usize, usize)]) -> f64 {
    if assignments.len() < 2 {
        return 1.0;
    }
    let contingency = contingency(assignments);
    let mut true_counts = BTreeMap::new();
    let mut predicted_counts = BTreeMap::new();
    for &(true_id, predicted_id) in assignments {
        *true_counts.entry(true_id).or_insert(0_usize) += 1;
        *predicted_counts.entry(predicted_id).or_insert(0_usize) += 1;
    }
    let sum_cells: f64 = contingency.values().map(|count| choose_two(*count)).sum();
    let sum_true: f64 = true_counts.values().map(|count| choose_two(*count)).sum();
    let sum_predicted: f64 = predicted_counts
        .values()
        .map(|count| choose_two(*count))
        .sum();
    let total_pairs = choose_two(assignments.len());
    let expected = sum_true * sum_predicted / total_pairs;
    let maximum = 0.5 * (sum_true + sum_predicted);
    if (maximum - expected).abs() < f64::EPSILON {
        if (sum_cells - maximum).abs() < f64::EPSILON {
            1.0
        } else {
            0.0
        }
    } else {
        (sum_cells - expected) / (maximum - expected)
    }
}

fn normalized_mutual_information(assignments: &[(usize, usize)]) -> f64 {
    if assignments.is_empty() {
        return 1.0;
    }
    let total = count_as_f64(assignments.len());
    let contingency = contingency(assignments);
    let mut true_counts = BTreeMap::new();
    let mut predicted_counts = BTreeMap::new();
    for &(true_id, predicted_id) in assignments {
        *true_counts.entry(true_id).or_insert(0_usize) += 1;
        *predicted_counts.entry(predicted_id).or_insert(0_usize) += 1;
    }
    let mut mutual_information = 0.0;
    for ((true_id, predicted_id), count) in contingency {
        let cell = count_as_f64(count);
        mutual_information += (cell / total)
            * ((cell * total)
                / (count_as_f64(true_counts[&true_id])
                    * count_as_f64(predicted_counts[&predicted_id])))
            .ln();
    }
    let true_entropy = entropy(true_counts.values().copied(), total);
    let predicted_entropy = entropy(predicted_counts.values().copied(), total);
    if true_entropy == 0.0 && predicted_entropy == 0.0 {
        1.0
    } else if true_entropy == 0.0 || predicted_entropy == 0.0 {
        0.0
    } else {
        mutual_information / (true_entropy * predicted_entropy).sqrt()
    }
}

fn contingency(assignments: &[(usize, usize)]) -> BTreeMap<(usize, usize), usize> {
    let mut output = BTreeMap::new();
    for assignment in assignments {
        *output.entry(*assignment).or_insert(0) += 1;
    }
    output
}

fn entropy(counts: impl Iterator<Item = usize>, total: f64) -> f64 {
    counts
        .map(|count| count_as_f64(count) / total)
        .filter(|probability| *probability > 0.0)
        .map(|probability| -probability * probability.ln())
        .sum()
}

fn choose_two(value: usize) -> f64 {
    let value = count_as_f64(value);
    value * (value - 1.0) / 2.0
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        count_as_f64(numerator) / count_as_f64(denominator)
    }
}

fn count_as_f64(value: usize) -> f64 {
    u32::try_from(value).map_or(f64::from(u32::MAX), f64::from)
}

#[derive(Debug)]
struct UnionFind {
    parent: Vec<usize>,
    rank: Vec<u8>,
}

impl UnionFind {
    fn new(size: usize) -> Self {
        Self {
            parent: (0..size).collect(),
            rank: vec![0; size],
        }
    }

    fn root(&mut self, index: usize) -> usize {
        if self.parent[index] != index {
            self.parent[index] = self.root(self.parent[index]);
        }
        self.parent[index]
    }

    fn join(&mut self, left: usize, right: usize) {
        let left_root = self.root(left);
        let right_root = self.root(right);
        if left_root == right_root {
            return;
        }
        match self.rank[left_root].cmp(&self.rank[right_root]) {
            std::cmp::Ordering::Less => self.parent[left_root] = right_root,
            std::cmp::Ordering::Greater => self.parent[right_root] = left_root,
            std::cmp::Ordering::Equal => {
                self.parent[right_root] = left_root;
                self.rank[left_root] = self.rank[left_root].saturating_add(1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;

    fn agent(value: u128) -> AgentId {
        AgentId(Uuid::from_u128(value))
    }

    fn pair(left: u128, right: u128, score: f64) -> PairScore {
        PairScore {
            left: agent(left),
            right: agent(right),
            timing: score,
            amount: score,
            sequence: score,
            destination: score,
            synchrony: score,
            funding: score,
            funding_round: score,
            funding_batch: score,
            route: score,
            balance_rank: score,
            composite: score,
        }
    }

    #[test]
    fn perfect_fixture_scores_one() -> Result<(), String> {
        let scores = vec![
            pair(1, 2, 0.9),
            pair(3, 4, 0.9),
            pair(1, 3, 0.1),
            pair(1, 4, 0.1),
            pair(2, 3, 0.1),
            pair(2, 4, 0.1),
        ];
        let truth = GroundTruth {
            controllers: [
                (agent(1), "a"),
                (agent(2), "a"),
                (agent(3), "b"),
                (agent(4), "b"),
            ]
            .into_iter()
            .map(|(agent, label)| (agent, label.to_owned()))
            .collect(),
        };
        let metrics = evaluate_scores(&scores, &truth, 0.5)?;
        assert!((metrics.binary.roc_auc - 1.0).abs() < f64::EPSILON);
        assert!((metrics.binary.f1 - 1.0).abs() < f64::EPSILON);
        assert!((metrics.clustering.adjusted_rand - 1.0).abs() < 1e-12);
        assert!((metrics.clustering.normalized_mutual_information - 1.0).abs() < 1e-12);
        Ok(())
    }

    #[test]
    fn identical_uninformative_scores_remain_at_chance() -> Result<(), String> {
        let scores = vec![
            pair(1, 2, 0.5),
            pair(1, 3, 0.5),
            pair(1, 4, 0.5),
            pair(2, 3, 0.5),
            pair(2, 4, 0.5),
            pair(3, 4, 0.5),
        ];
        let truth = GroundTruth {
            controllers: [
                (agent(1), "shuffled-b"),
                (agent(2), "shuffled-a"),
                (agent(3), "shuffled-b"),
                (agent(4), "shuffled-a"),
            ]
            .into_iter()
            .map(|(agent, label)| (agent, label.to_owned()))
            .collect(),
        };
        let metrics = evaluate_scores(&scores, &truth, 0.5)?;
        assert!((metrics.binary.roc_auc - 0.5).abs() < f64::EPSILON);
        Ok(())
    }
}
