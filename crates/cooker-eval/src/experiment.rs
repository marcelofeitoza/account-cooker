//! Reproducible naive, independent, and persona/session comparisons.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{Duration, TimeZone, Utc};
use cooker_core::{ActionId, ActionKind, AgentId, RunId, TraceEvent, TraceOutcome};
use rand::{Rng, SeedableRng, seq::SliceRandom};
use rand_chacha::ChaCha12Rng;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    AttackWeights, EvaluationMetrics, FeatureConfig, FeatureFamily, GroundTruth,
    ObservationDataset, PairScore, evaluate_scores, extract_features, score_pairs,
};

/// Planner family compared under the same fleet and action budget.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlannerVariant {
    /// Fixed controller-level cadence and uniform action selection.
    NaiveUniform,
    /// Independent timing with weighted actions but no sessions.
    IndependentWeighted,
    /// Independent persona windows, sessions, bursts, and sequences.
    PersonaSession,
}

/// One feature-family removal from the composite attacker.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "feature")]
pub enum Ablation {
    /// Full fixed attacker.
    None,
    /// Remove one feature family and renormalize the remainder.
    Without(FeatureFamily),
}

/// Equal-budget comparative experiment settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExperimentConfig {
    /// Independent controller groups.
    pub controllers: usize,
    /// Agents controlled by each group.
    pub agents_per_controller: usize,
    /// Virtual UTC days.
    pub days: u32,
    /// Exact events per agent and day for every variant.
    pub events_per_agent_per_day: u32,
    /// Held-out master seeds.
    pub seeds: Vec<u64>,
    /// Label-independent linkage threshold.
    pub threshold: f64,
    /// Feature extraction configuration.
    pub features: FeatureConfig,
    /// Fixed composite weights.
    pub weights: AttackWeights,
    /// Requested composite ablations.
    pub ablations: Vec<Ablation>,
}

impl Default for ExperimentConfig {
    fn default() -> Self {
        Self {
            controllers: 10,
            agents_per_controller: 10,
            days: 30,
            events_per_agent_per_day: 2,
            seeds: vec![11, 23, 37, 51, 71],
            threshold: 0.55,
            features: FeatureConfig::default(),
            weights: AttackWeights::default(),
            ablations: vec![
                Ablation::None,
                Ablation::Without(FeatureFamily::Timing),
                Ablation::Without(FeatureFamily::Amount),
                Ablation::Without(FeatureFamily::Sequence),
                Ablation::Without(FeatureFamily::Synchrony),
                Ablation::Without(FeatureFamily::Funding),
            ],
        }
    }
}

impl ExperimentConfig {
    /// Validate that comparisons contain both positive and negative pairs.
    ///
    /// # Errors
    ///
    /// Returns an error for unsafe sizes, duplicate seeds, an invalid threshold, or no full run.
    pub fn validate(&self) -> Result<(), String> {
        if self.controllers < 2 {
            return Err("at least two controllers are required".to_owned());
        }
        if self.controllers > 10_000 || self.agents_per_controller > 10_000 {
            return Err("controller and agent counts are limited to 10,000".to_owned());
        }
        if self.agents_per_controller < 2 {
            return Err("at least two agents per controller are required".to_owned());
        }
        if self.days == 0 || self.events_per_agent_per_day == 0 {
            return Err("days and events_per_agent_per_day must be positive".to_owned());
        }
        if self.seeds.len() < 5 {
            return Err("at least five held-out seeds are required".to_owned());
        }
        if self.seeds.iter().copied().collect::<BTreeSet<_>>().len() != self.seeds.len() {
            return Err("held-out seeds must be unique".to_owned());
        }
        if !(0.0..=1.0).contains(&self.threshold) {
            return Err("threshold must be between zero and one".to_owned());
        }
        if !self.ablations.contains(&Ablation::None) {
            return Err("ablations must include the full attacker".to_owned());
        }
        Ok(())
    }
}

/// One seed, planner, and ablation result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SeedResult {
    /// Planner family.
    pub planner: PlannerVariant,
    /// Held-out seed.
    pub seed: u64,
    /// Composite ablation.
    pub ablation: Ablation,
    /// Stable hash of normalized observations.
    pub trace_hash: String,
    /// Number of observation rows.
    pub events: usize,
    /// Number of unordered scored pairs.
    pub pairs: usize,
    /// Observable action counts by family.
    pub action_counts: BTreeMap<ActionKind, usize>,
    /// Actions rejected during generation; synthetic equal-budget runs currently reject none.
    pub rejected_actions: usize,
    /// Composite attacker metrics.
    pub composite: EvaluationMetrics,
    /// Each attacker feature evaluated independently.
    pub per_feature: BTreeMap<FeatureFamily, EvaluationMetrics>,
    /// Within-controller and between-controller score distance for every feature.
    pub feature_separation: BTreeMap<FeatureFamily, FeatureSeparation>,
}

/// Label-stage distance between controlled and unrelated wallet pairs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FeatureSeparation {
    /// Mean feature score among pairs with the same controller.
    pub within_controller_mean: f64,
    /// Mean feature score among pairs with different controllers.
    pub between_controller_mean: f64,
    /// Signed difference `within_controller_mean - between_controller_mean`.
    pub separation: f64,
}

/// Mean and observed range across held-out seeds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MetricRange {
    /// Arithmetic mean.
    pub mean: f64,
    /// Minimum held-out value.
    pub min: f64,
    /// Maximum held-out value.
    pub max: f64,
    /// Sample standard deviation across held-out seeds.
    pub standard_deviation: f64,
    /// Lower normal-approximation 95% confidence bound.
    pub confidence_95_low: f64,
    /// Upper normal-approximation 95% confidence bound.
    pub confidence_95_high: f64,
}

/// Aggregated composite metrics for one planner and ablation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AggregateResult {
    /// Planner family.
    pub planner: PlannerVariant,
    /// Composite ablation.
    pub ablation: Ablation,
    /// ROC AUC summary.
    pub roc_auc: MetricRange,
    /// F1 summary.
    pub f1: MetricRange,
    /// Precision-at-K summary.
    pub precision_at_k: MetricRange,
    /// Adjusted Rand summary.
    pub adjusted_rand: MetricRange,
    /// Normalized mutual information summary.
    pub normalized_mutual_information: MetricRange,
}

/// Complete reproducible comparative result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExperimentResult {
    /// Exact settings used.
    pub config: ExperimentConfig,
    /// Per-seed results with no hidden exclusions.
    pub seeds: Vec<SeedResult>,
    /// Mean, minimum, and maximum summaries.
    pub aggregates: Vec<AggregateResult>,
    /// Explicit interpretation bound that accompanies every report.
    pub limitation: String,
}

/// Run all planner variants, seeds, features, and configured ablations.
///
/// # Errors
///
/// Returns an error when configuration, serialization, feature labels, or metrics are invalid.
pub fn run_experiment(config: ExperimentConfig) -> Result<ExperimentResult, String> {
    config.validate()?;
    let variants = [
        PlannerVariant::NaiveUniform,
        PlannerVariant::IndependentWeighted,
        PlannerVariant::PersonaSession,
    ];
    let mut seed_results = Vec::new();

    for variant in variants {
        for &seed in &config.seeds {
            let (mut observations, truth) = generate_dataset(&config, variant, seed)?;
            observations.normalize();
            let trace_bytes = serde_json::to_vec(&observations)
                .map_err(|error| format!("failed to serialize trace: {error}"))?;
            let trace_hash = blake3::hash(&trace_bytes).to_hex().to_string();
            let features = extract_features(&observations.events, config.features);
            let full_pairs = score_pairs(&features, config.weights);
            let per_feature = per_feature_metrics(&full_pairs, &truth, config.threshold)?;
            let feature_separation = feature_separations(&full_pairs, &truth)?;
            let mut action_counts = BTreeMap::new();
            for event in &observations.events {
                *action_counts.entry(event.action_kind).or_insert(0) += 1;
            }

            for &ablation in &config.ablations {
                let weights = match ablation {
                    Ablation::None => config.weights,
                    Ablation::Without(family) => config.weights.without(family),
                };
                let pairs = score_pairs(&features, weights);
                let composite = evaluate_scores(&pairs, &truth, config.threshold)?;
                seed_results.push(SeedResult {
                    planner: variant,
                    seed,
                    ablation,
                    trace_hash: trace_hash.clone(),
                    events: observations.events.len(),
                    pairs: pairs.len(),
                    action_counts: action_counts.clone(),
                    rejected_actions: 0,
                    composite,
                    per_feature: per_feature.clone(),
                    feature_separation: feature_separation.clone(),
                });
            }
        }
    }

    let aggregates = aggregate(&seed_results);
    Ok(ExperimentResult {
        config,
        seeds: seed_results,
        aggregates,
        limitation: "The common-funder graph remains directly observable; lower behavioral linkage scores do not establish transaction-graph anonymity.".to_owned(),
    })
}

fn generate_dataset(
    config: &ExperimentConfig,
    variant: PlannerVariant,
    seed: u64,
) -> Result<(ObservationDataset, GroundTruth), String> {
    let start = Utc
        .with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
        .single()
        .ok_or_else(|| "invalid experiment epoch".to_owned())?;
    let run_id = RunId(Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("noise-eval-{variant:?}-{seed}").as_bytes(),
    ));
    let mut events = Vec::with_capacity(
        config
            .controllers
            .saturating_mul(config.agents_per_controller)
            .saturating_mul(config.days as usize)
            .saturating_mul(config.events_per_agent_per_day as usize),
    );
    let mut controllers = BTreeMap::new();

    for controller_index in 0..config.controllers {
        let controller = format!("controller-{controller_index:03}");
        for agent_index in 0..config.agents_per_controller {
            let agent_id = AgentId(Uuid::new_v5(
                &Uuid::NAMESPACE_OID,
                format!("noise-agent-{controller_index}-{agent_index}").as_bytes(),
            ));
            controllers.insert(agent_id, controller.clone());
            let decision_seed = decision_seed(seed, agent_id, variant);
            let mut rng = ChaCha12Rng::from_seed(decision_seed);
            let timezone_offset = rng.gen_range(-10_i64..=10) * 3_600;
            let session_hour = rng.gen_range(7_i64..=21);
            let mut previous_kind = ActionKind::NativeTransfer;
            let mut sequence = 0_u64;

            for day in 0..config.days {
                for event_index in 0..config.events_per_agent_per_day {
                    let (seconds, kind, amount, destination, route) = generate_observation(
                        variant,
                        controller_index,
                        agent_index,
                        event_index,
                        timezone_offset,
                        session_hour,
                        previous_kind,
                        &mut rng,
                    );
                    previous_kind = kind;
                    let timestamp = start
                        + Duration::days(i64::from(day))
                        + Duration::seconds(seconds.clamp(0, 86_399));
                    let action_id = ActionId::derive(run_id, agent_id, sequence, "eval-v1");
                    let mut attributes = BTreeMap::new();
                    attributes.insert("funder".to_owned(), format!("funder-{controller_index:03}"));
                    attributes.insert("route".to_owned(), route);
                    attributes.insert(
                        "balance_rank".to_owned(),
                        format!(
                            "{}",
                            controller_index * config.agents_per_controller + agent_index
                        ),
                    );
                    events.push(TraceEvent {
                        schema_version: TraceEvent::SCHEMA_VERSION,
                        run_id,
                        agent_id,
                        action_id,
                        sequence,
                        action_kind: kind,
                        scheduled_at: timestamp,
                        observed_at: Some(timestamp),
                        amount,
                        destination: Some(destination),
                        signature: None,
                        outcome: TraceOutcome::Confirmed,
                        attributes,
                    });
                    sequence = sequence.saturating_add(1);
                }
            }
        }
    }

    Ok((ObservationDataset { events }, GroundTruth { controllers }))
}

#[allow(clippy::too_many_arguments)]
fn generate_observation(
    variant: PlannerVariant,
    controller: usize,
    _agent: usize,
    event_index: u32,
    timezone_offset: i64,
    session_hour: i64,
    previous_kind: ActionKind,
    rng: &mut ChaCha12Rng,
) -> (i64, ActionKind, u64, String, String) {
    const KINDS: [ActionKind; 4] = [
        ActionKind::NativeTransfer,
        ActionKind::SplTransfer,
        ActionKind::JupiterSwap,
        ActionKind::StakeLifecycle,
    ];
    match variant {
        PlannerVariant::NaiveUniform => {
            let controller_offset = i64::try_from(controller).map_or(i64::MAX, |value| value * 240);
            let seconds = 9 * 3_600 + controller_offset + i64::from(event_index) * 1_800;
            let kind = KINDS[(event_index as usize + controller) % KINDS.len()];
            let amount = (controller as u64 + 1) * 1_000_000;
            (
                seconds,
                kind,
                amount,
                format!("controller-destination-{controller:03}"),
                format!("route-{}", controller % 2),
            )
        }
        PlannerVariant::IndependentWeighted => {
            let seconds = rng.gen_range(0_i64..86_400);
            let roll = rng.gen_range(0_u8..100);
            let kind = if roll < 45 {
                ActionKind::NativeTransfer
            } else if roll < 65 {
                ActionKind::SplTransfer
            } else if roll < 90 {
                ActionKind::JupiterSwap
            } else {
                ActionKind::StakeLifecycle
            };
            let base = 10_u64.pow(rng.gen_range(4_u32..=8));
            let amount = base.saturating_add(rng.gen_range(0..base.max(2) / 2));
            (
                seconds,
                kind,
                amount,
                format!("shared-destination-{:03}", rng.gen_range(0..32)),
                format!("route-{}", rng.gen_range(0..4)),
            )
        }
        PlannerVariant::PersonaSession => {
            let session_start = session_hour * 3_600 - timezone_offset;
            let burst_offset = i64::from(event_index) * rng.gen_range(120_i64..=2_400);
            let seconds = session_start + burst_offset + rng.gen_range(-1_200_i64..=1_200);
            let kind = markov_kind(previous_kind, rng);
            let magnitude = 10_u64.pow(rng.gen_range(3_u32..=8));
            let jitter = rng.gen_range(0..magnitude.max(2));
            let mut amount = magnitude.saturating_add(jitter);
            if rng.gen_bool(0.18) {
                amount = amount / 1_000 * 1_000;
            }
            (
                seconds,
                kind,
                amount.max(1),
                format!("shared-destination-{:03}", rng.gen_range(0..64)),
                format!("route-{}", rng.gen_range(0..5)),
            )
        }
    }
}

fn markov_kind(previous: ActionKind, rng: &mut ChaCha12Rng) -> ActionKind {
    let mut choices = match previous {
        ActionKind::NativeTransfer | ActionKind::StakeLifecycle | ActionKind::Idle => [
            ActionKind::NativeTransfer,
            ActionKind::JupiterSwap,
            ActionKind::SplTransfer,
            ActionKind::StakeLifecycle,
        ],
        ActionKind::SplTransfer => [
            ActionKind::SplTransfer,
            ActionKind::NativeTransfer,
            ActionKind::JupiterSwap,
            ActionKind::StakeLifecycle,
        ],
        ActionKind::JupiterSwap => [
            ActionKind::JupiterSwap,
            ActionKind::SplTransfer,
            ActionKind::NativeTransfer,
            ActionKind::StakeLifecycle,
        ],
    };
    choices.shuffle(rng);
    choices[0]
}

fn decision_seed(seed: u64, agent: AgentId, variant: PlannerVariant) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&seed.to_le_bytes());
    hasher.update(agent.0.as_bytes());
    hasher.update(format!("{variant:?}").as_bytes());
    *hasher.finalize().as_bytes()
}

fn per_feature_metrics(
    pairs: &[PairScore],
    truth: &GroundTruth,
    threshold: f64,
) -> Result<BTreeMap<FeatureFamily, EvaluationMetrics>, String> {
    let mut output = BTreeMap::new();
    for family in [
        FeatureFamily::Timing,
        FeatureFamily::Amount,
        FeatureFamily::Sequence,
        FeatureFamily::Destination,
        FeatureFamily::Synchrony,
        FeatureFamily::Funding,
        FeatureFamily::Route,
        FeatureFamily::BalanceRank,
    ] {
        let family_pairs: Vec<_> = pairs
            .iter()
            .map(|pair| {
                let mut pair = pair.clone();
                pair.composite = match family {
                    FeatureFamily::Timing => pair.timing,
                    FeatureFamily::Amount => pair.amount,
                    FeatureFamily::Sequence => pair.sequence,
                    FeatureFamily::Destination => pair.destination,
                    FeatureFamily::Synchrony => pair.synchrony,
                    FeatureFamily::Funding => pair.funding,
                    FeatureFamily::Route => pair.route,
                    FeatureFamily::BalanceRank => pair.balance_rank,
                };
                pair
            })
            .collect();
        output.insert(family, evaluate_scores(&family_pairs, truth, threshold)?);
    }
    Ok(output)
}

fn feature_separations(
    pairs: &[PairScore],
    truth: &GroundTruth,
) -> Result<BTreeMap<FeatureFamily, FeatureSeparation>, String> {
    let mut output = BTreeMap::new();
    for family in [
        FeatureFamily::Timing,
        FeatureFamily::Amount,
        FeatureFamily::Sequence,
        FeatureFamily::Destination,
        FeatureFamily::Synchrony,
        FeatureFamily::Funding,
        FeatureFamily::Route,
        FeatureFamily::BalanceRank,
    ] {
        let mut within = Vec::new();
        let mut between = Vec::new();
        for pair in pairs {
            let same = truth
                .same_controller(pair.left, pair.right)
                .ok_or_else(|| "feature pair is missing a controller label".to_owned())?;
            let value = pair_feature(pair, family);
            if same {
                within.push(value);
            } else {
                between.push(value);
            }
        }
        let within_controller_mean = mean(&within);
        let between_controller_mean = mean(&between);
        output.insert(
            family,
            FeatureSeparation {
                within_controller_mean,
                between_controller_mean,
                separation: within_controller_mean - between_controller_mean,
            },
        );
    }
    Ok(output)
}

fn pair_feature(pair: &PairScore, family: FeatureFamily) -> f64 {
    match family {
        FeatureFamily::Timing => pair.timing,
        FeatureFamily::Amount => pair.amount,
        FeatureFamily::Sequence => pair.sequence,
        FeatureFamily::Destination => pair.destination,
        FeatureFamily::Synchrony => pair.synchrony,
        FeatureFamily::Funding => pair.funding,
        FeatureFamily::Route => pair.route,
        FeatureFamily::BalanceRank => pair.balance_rank,
    }
}

fn aggregate(results: &[SeedResult]) -> Vec<AggregateResult> {
    let mut groups: BTreeMap<(PlannerVariant, Ablation), Vec<&SeedResult>> = BTreeMap::new();
    for result in results {
        groups
            .entry((result.planner, result.ablation))
            .or_default()
            .push(result);
    }
    groups
        .into_iter()
        .map(|((planner, ablation), results)| AggregateResult {
            planner,
            ablation,
            roc_auc: metric_range(results.iter().map(|result| result.composite.binary.roc_auc)),
            f1: metric_range(results.iter().map(|result| result.composite.binary.f1)),
            precision_at_k: metric_range(
                results
                    .iter()
                    .map(|result| result.composite.binary.precision_at_k),
            ),
            adjusted_rand: metric_range(
                results
                    .iter()
                    .map(|result| result.composite.clustering.adjusted_rand),
            ),
            normalized_mutual_information: metric_range(
                results
                    .iter()
                    .map(|result| result.composite.clustering.normalized_mutual_information),
            ),
        })
        .collect()
}

fn metric_range(values: impl Iterator<Item = f64>) -> MetricRange {
    let values: Vec<_> = values.collect();
    let denominator = u32::try_from(values.len().max(1)).map_or(f64::from(u32::MAX), f64::from);
    let mean = values.iter().sum::<f64>() / denominator;
    let min = values.iter().copied().reduce(f64::min).unwrap_or(0.0);
    let max = values.iter().copied().reduce(f64::max).unwrap_or(0.0);
    let standard_deviation = if values.len() > 1 {
        let squared_error = values
            .iter()
            .map(|value| (value - mean).powi(2))
            .sum::<f64>();
        let denominator = u32::try_from(values.len() - 1).map_or(f64::from(u32::MAX), f64::from);
        (squared_error / denominator).sqrt()
    } else {
        0.0
    };
    let sample_size = u32::try_from(values.len().max(1)).map_or(f64::from(u32::MAX), f64::from);
    let margin = 1.96 * standard_deviation / sample_size.sqrt();
    MetricRange {
        mean,
        min,
        max,
        standard_deviation,
        confidence_95_low: mean - margin,
        confidence_95_high: mean + margin,
    }
}

fn mean(values: &[f64]) -> f64 {
    let denominator = u32::try_from(values.len().max(1)).map_or(f64::from(u32::MAX), f64::from);
    values.iter().sum::<f64>() / denominator
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compact_config() -> ExperimentConfig {
        ExperimentConfig {
            controllers: 3,
            agents_per_controller: 3,
            days: 2,
            events_per_agent_per_day: 2,
            seeds: vec![1, 2, 3, 4, 5],
            ablations: vec![Ablation::None, Ablation::Without(FeatureFamily::Funding)],
            ..ExperimentConfig::default()
        }
    }

    #[test]
    fn experiment_is_byte_deterministic() -> Result<(), Box<dyn std::error::Error>> {
        let first = run_experiment(compact_config())?;
        let second = run_experiment(compact_config())?;
        assert_eq!(serde_json::to_vec(&first)?, serde_json::to_vec(&second)?);
        Ok(())
    }

    #[test]
    fn equal_action_budgets_are_preserved() -> Result<(), String> {
        let config = compact_config();
        let expected = config.controllers
            * config.agents_per_controller
            * config.days as usize
            * config.events_per_agent_per_day as usize;
        let result = run_experiment(config)?;
        assert!(result.seeds.iter().all(|seed| seed.events == expected));
        Ok(())
    }

    #[test]
    fn common_funder_limit_remains_measurable() -> Result<(), String> {
        let result = run_experiment(compact_config())?;
        for seed in result
            .seeds
            .iter()
            .filter(|seed| seed.ablation == Ablation::None)
        {
            assert!(
                (seed.per_feature[&FeatureFamily::Funding].binary.roc_auc - 1.0).abs()
                    < f64::EPSILON
            );
        }
        Ok(())
    }

    #[test]
    fn controller_labels_cannot_change_extracted_scores() -> Result<(), String> {
        let config = compact_config();
        let (observations, mut truth) =
            generate_dataset(&config, PlannerVariant::PersonaSession, 9)?;
        let features = extract_features(&observations.events, config.features);
        let before = score_pairs(&features, config.weights);
        for label in truth.controllers.values_mut() {
            *label = "relabeled-after-extraction".to_owned();
        }
        let after = score_pairs(&features, config.weights);
        assert_eq!(before, after);
        Ok(())
    }
}
