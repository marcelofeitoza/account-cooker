//! Adversarial measurement for observation-only account traces.

#![forbid(unsafe_code)]

mod attacks;
mod dataset;
mod experiment;
mod features;
mod metrics;
mod report;

pub use attacks::{AttackWeights, FeatureFamily, PairScore, score_pairs};
pub use dataset::{GroundTruth, ObservationDataset};
pub use experiment::{
    Ablation, ExperimentConfig, ExperimentResult, FeatureSeparation, FundingModel,
    FundingObservation, FundingSummary, PlannerVariant, run_experiment,
};
pub use features::{FeatureConfig, FeatureSet, extract_features};
pub use metrics::{BinaryMetrics, ClusteringMetrics, EvaluationMetrics, evaluate_scores};
pub use report::{render_csv, render_markdown};
