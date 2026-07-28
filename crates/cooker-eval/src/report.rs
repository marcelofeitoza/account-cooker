//! Stable human-readable and tabular experiment reports.

use std::fmt::Write;

use crate::{Ablation, ExperimentResult, experiment::SeedResult};

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
    let _ = writeln!(
        output,
        "- Funding schemes: {:?}\n- Shared disbursers: {}\n- Round length (hours): {}\n- Top-ups per account: {}\n- Uniform denomination (lamports): {}",
        result.config.funding_schemes,
        result.config.funding.disbursers,
        result.config.funding.round_hours,
        result.config.funding.top_ups_per_account,
        result.config.funding.denomination_lamports,
    );
    write_funding_provenance(&mut output, result);
    output.push_str("\n## Composite Results\n\n");
    output.push_str("| Funding scheme | Planner | Ablation | ROC AUC mean [95% CI] | F1 mean [95% CI] | Precision@K mean | ARI mean | NMI mean |\n");
    output.push_str("|---|---|---|---:|---:|---:|---:|---:|\n");
    for aggregate in &result.aggregates {
        let _ = writeln!(
            output,
            "| {:?} | {:?} | {} | {:.4} [{:.4}, {:.4}] | {:.4} [{:.4}, {:.4}] | {:.4} | {:.4} | {:.4} |",
            aggregate.funding_scheme,
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
    output.push_str("| Funding scheme | Planner | Seed | Ablation | Trace hash | Events | Pairs | ROC AUC | F1 | Precision@K | ARI | NMI |\n");
    output.push_str("|---|---|---:|---|---|---:|---:|---:|---:|---:|---:|---:|\n");
    for seed in &result.seeds {
        let _ = writeln!(
            output,
            "| {:?} | {:?} | {} | {} | `{}` | {} | {} | {:.4} | {:.4} | {:.4} | {:.4} | {:.4} |",
            seed.funding_scheme,
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
        "| Funding scheme | Planner | Seed | Feature | Within mean | Between mean | Separation | ROC AUC |\n",
    );
    output.push_str("|---|---|---:|---|---:|---:|---:|---:|\n");
    for seed in result
        .seeds
        .iter()
        .filter(|seed| seed.ablation == Ablation::None)
    {
        for (feature, separation) in &seed.feature_separation {
            let metrics = &seed.per_feature[feature];
            let _ = writeln!(
                output,
                "| {:?} | {:?} | {} | {:?} | {:.4} | {:.4} | {:.4} | {:.4} |",
                seed.funding_scheme,
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

/// Write the funding-provenance comparison and the schedule cost that produced it.
fn write_funding_provenance(output: &mut String, result: &ExperimentResult) {
    output.push_str("\n## Funding Provenance\n\n");
    output.push_str(
        "Every scheme runs the same behavior, budgets, attacker, threshold, and participation \
         schedule. Only the payer assignment changes, so any movement in these columns is \
         attributable to the payer assignment and nothing else. Three funding attacks are \
         reported. Funding is the common-funder overlap. Funding round is the same overlap after \
         it also reads the batching round. Funding batch is the scheme-aware attack: it assumes \
         the adversary knows accounts are dealt into pooled batches and weights a shared \
         payer-and-round batch by how few accounts were in it, so a shared batch of two counts \
         far more than a shared batch of twenty. All three read only what the chain publishes.\n\n",
    );
    output.push_str(
        "| Funding scheme | Planner | Funding AUC mean [95% CI] | Funding round AUC mean | \
         Funding batch AUC mean | Funding separation mean | Composite AUC mean |\n",
    );
    output.push_str("|---|---|---:|---:|---:|---:|---:|\n");
    for summary in &result.funding_summary {
        let _ = writeln!(
            output,
            "| {:?} | {:?} | {:.4} [{:.4}, {:.4}] | {:.4} | {:.4} | {:.4} | {:.4} |",
            summary.funding_scheme,
            summary.planner,
            summary.funding_roc_auc.mean,
            summary.funding_roc_auc.confidence_95_low,
            summary.funding_roc_auc.confidence_95_high,
            summary.funding_round_roc_auc.mean,
            summary.funding_batch_roc_auc.mean,
            summary.funding_separation.mean,
            summary.composite_roc_auc.mean,
        );
    }
    output.push_str(
        "\nThe fixed composite holds the funding-round and funding-batch weights at zero so \
         composite numbers stay comparable with the pre-mitigation baseline. That choice is \
         conservative for the pooled arm: both extra families sit near chance there, so folding \
         them into the composite would dilute the informative families and push the pooled \
         composite lower than reported.\n",
    );
    output.push_str("\n### Funding Schedule Cost\n\n");
    output.push_str(
        "Transfer counts are equal across schemes by construction, because every scheme replays \
         one shared participation schedule. The cost of pooling is therefore read against \
         operator practice rather than against another row here: a naive operator provisions \
         each account once from one wallet, so the pooled schedule multiplies funding transfers \
         by the top-ups-per-account setting and adds the operator's own deposits into the pool, \
         which this evaluator does not model. The smallest-batch column is the residual: a round \
         and payer batch with one recipient gives that transfer no cover at all.\n\n",
    );
    output.push_str(
        "| Funding scheme | Seed | Transfers | Distinct payers | Rounds | Mean batch recipients \
         | Smallest batch | Mean observed payers per account |\n",
    );
    output.push_str("|---|---:|---:|---:|---:|---:|---:|---:|\n");
    for seed in schedule_rows(result) {
        let _ = writeln!(
            output,
            "| {:?} | {} | {} | {} | {} | {:.2} | {} | {:.2} |",
            seed.funding_scheme,
            seed.seed,
            seed.funding.transfers,
            seed.funding.funders,
            seed.funding.rounds,
            seed.funding.mean_batch_recipients,
            seed.funding.min_batch_recipients,
            seed.funding.mean_observed_funders,
        );
    }
}

/// Render one stable CSV row per seed and ablation.
#[must_use]
pub fn render_csv(result: &ExperimentResult) -> String {
    let mut output = String::from(
        "funding_scheme,planner,seed,ablation,metric,trace_hash,events,pairs,rejected_actions,roc_auc,precision,recall,f1,precision_at_k,adjusted_rand,nmi,within_mean,between_mean,separation\n",
    );
    for seed in &result.seeds {
        let _ = writeln!(
            output,
            "{:?},{:?},{},{},composite,{},{},{},{},{:.8},{:.8},{:.8},{:.8},{:.8},{:.8},{:.8},,,",
            seed.funding_scheme,
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
                "{:?},{:?},{},{},{:?},{},{},{},{},{:.8},{:.8},{:.8},{:.8},{:.8},{:.8},{:.8},{:.8},{:.8},{:.8}",
                seed.funding_scheme,
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

fn schedule_rows(result: &ExperimentResult) -> Vec<&SeedResult> {
    let mut seen = Vec::new();
    let mut output = Vec::new();
    for seed in &result.seeds {
        let key = (seed.funding_scheme, seed.seed);
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        output.push(seed);
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
        assert!(markdown.contains("do not establish transaction-graph anonymity"));
        assert!(markdown.contains("## Funding Provenance"));
        assert!(markdown.contains("PooledMixedRounds"));
        assert!(markdown.contains("| 5 |"));
        assert_eq!(
            render_csv(&result).lines().count(),
            result.seeds.len() * 11 + 1
        );
        Ok(())
    }
}
