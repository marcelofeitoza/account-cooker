//! Stable human-readable and tabular experiment reports.

use std::fmt::Write;

use crate::{Ablation, ExperimentResult};

/// Render aggregate results and limitations as Markdown.
#[must_use]
pub fn render_markdown(result: &ExperimentResult) -> String {
    let mut output = String::new();
    output.push_str("# Account Cooker Evaluation\n\n");
    output.push_str("## Experiment\n\n");
    let _ = writeln!(
        output,
        "- Controllers: {}\n- Agents per controller: {}\n- Days: {}\n- Events per agent/day: {}\n- Held-out seeds: {:?}\n- Fixed threshold: {:.3}",
        result.config.controllers,
        result.config.agents_per_controller,
        result.config.days,
        result.config.events_per_agent_per_day,
        result.config.seeds,
        result.config.threshold
    );
    output.push_str("\n## Composite Results\n\n");
    output.push_str("| Planner | Ablation | ROC AUC mean [95% CI] | F1 mean [95% CI] | Precision@K mean | ARI mean | NMI mean |\n");
    output.push_str("|---|---|---:|---:|---:|---:|---:|\n");
    for aggregate in &result.aggregates {
        let _ = writeln!(
            output,
            "| {:?} | {} | {:.4} [{:.4}, {:.4}] | {:.4} [{:.4}, {:.4}] | {:.4} | {:.4} | {:.4} |",
            aggregate.planner,
            ablation_name(aggregate.ablation),
            aggregate.roc_auc.mean,
            aggregate.roc_auc.confidence_95_low,
            aggregate.roc_auc.confidence_95_high,
            aggregate.f1.mean,
            aggregate.f1.confidence_95_low,
            aggregate.f1.confidence_95_high,
            aggregate.precision_at_k.mean,
            aggregate.adjusted_rand.mean,
            aggregate.normalized_mutual_information.mean,
        );
    }
    output.push_str("\n## Per-Seed Results\n\n");
    output.push_str("| Planner | Seed | Ablation | Trace hash | Events | Pairs | ROC AUC | F1 | Precision@K | ARI | NMI |\n");
    output.push_str("|---|---:|---|---|---:|---:|---:|---:|---:|---:|---:|\n");
    for seed in &result.seeds {
        let _ = writeln!(
            output,
            "| {:?} | {} | {} | `{}` | {} | {} | {:.4} | {:.4} | {:.4} | {:.4} | {:.4} |",
            seed.planner,
            seed.seed,
            ablation_name(seed.ablation),
            seed.trace_hash,
            seed.events,
            seed.pairs,
            seed.composite.binary.roc_auc,
            seed.composite.binary.f1,
            seed.composite.binary.precision_at_k,
            seed.composite.clustering.adjusted_rand,
            seed.composite.clustering.normalized_mutual_information,
        );
    }
    output.push_str("\n## Per-Feature Separation\n\n");
    output.push_str("Feature separation is the mean score for same-controller pairs minus the mean score for different-controller pairs.\n\n");
    output.push_str(
        "| Planner | Seed | Feature | Within mean | Between mean | Separation | ROC AUC |\n",
    );
    output.push_str("|---|---:|---|---:|---:|---:|---:|\n");
    for seed in result
        .seeds
        .iter()
        .filter(|seed| seed.ablation == Ablation::None)
    {
        for (feature, separation) in &seed.feature_separation {
            let metrics = &seed.per_feature[feature];
            let _ = writeln!(
                output,
                "| {:?} | {} | {:?} | {:.4} | {:.4} | {:.4} | {:.4} |",
                seed.planner,
                seed.seed,
                feature,
                separation.within_controller_mean,
                separation.between_controller_mean,
                separation.separation,
                metrics.binary.roc_auc,
            );
        }
    }
    output.push_str("\n## Interpretation Bound\n\n");
    output.push_str(&result.limitation);
    output.push('\n');
    output
}

/// Render one stable CSV row per seed and ablation.
#[must_use]
pub fn render_csv(result: &ExperimentResult) -> String {
    let mut output = String::from(
        "planner,seed,ablation,metric,trace_hash,events,pairs,rejected_actions,roc_auc,precision,recall,f1,precision_at_k,adjusted_rand,nmi,within_mean,between_mean,separation\n",
    );
    for seed in &result.seeds {
        let _ = writeln!(
            output,
            "{:?},{},{},composite,{},{},{},{},{:.8},{:.8},{:.8},{:.8},{:.8},{:.8},{:.8},,,",
            seed.planner,
            seed.seed,
            ablation_name(seed.ablation),
            seed.trace_hash,
            seed.events,
            seed.pairs,
            seed.rejected_actions,
            seed.composite.binary.roc_auc,
            seed.composite.binary.precision,
            seed.composite.binary.recall,
            seed.composite.binary.f1,
            seed.composite.binary.precision_at_k,
            seed.composite.clustering.adjusted_rand,
            seed.composite.clustering.normalized_mutual_information,
        );
        for (feature, metrics) in &seed.per_feature {
            let separation = &seed.feature_separation[feature];
            let _ = writeln!(
                output,
                "{:?},{},{},{:?},{},{},{},{},{:.8},{:.8},{:.8},{:.8},{:.8},{:.8},{:.8},{:.8},{:.8},{:.8}",
                seed.planner,
                seed.seed,
                ablation_name(seed.ablation),
                feature,
                seed.trace_hash,
                seed.events,
                seed.pairs,
                seed.rejected_actions,
                metrics.binary.roc_auc,
                metrics.binary.precision,
                metrics.binary.recall,
                metrics.binary.f1,
                metrics.binary.precision_at_k,
                metrics.clustering.adjusted_rand,
                metrics.clustering.normalized_mutual_information,
                separation.within_controller_mean,
                separation.between_controller_mean,
                separation.separation,
            );
        }
    }
    output
}

fn ablation_name(ablation: Ablation) -> String {
    match ablation {
        Ablation::None => "none".to_owned(),
        Ablation::Without(feature) => format!("without_{feature:?}").to_lowercase(),
    }
}

#[cfg(test)]
mod tests {
    use crate::{ExperimentConfig, run_experiment};

    use super::*;

    #[test]
    fn reports_include_limitations_and_all_seeds() -> Result<(), String> {
        let result = run_experiment(ExperimentConfig {
            controllers: 2,
            agents_per_controller: 2,
            days: 1,
            events_per_agent_per_day: 1,
            seeds: vec![1, 2, 3, 4, 5],
            ablations: vec![Ablation::None],
            ..ExperimentConfig::default()
        })?;
        let markdown = render_markdown(&result);
        assert!(markdown.contains("common-funder graph remains directly observable"));
        assert!(markdown.contains("| 5 |"));
        assert_eq!(
            render_csv(&result).lines().count(),
            result.seeds.len() * 9 + 1
        );
        Ok(())
    }
}
