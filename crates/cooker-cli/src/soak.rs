//! Canonical virtual soak execution and evidence verification.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{BufRead, BufReader, Write},
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Days, NaiveDate, Utc};
use cooker_core::{
    ActionKind, AgentId, SimulationArtifact, SimulationConfig, TraceEvent, TraceOutcome,
    run_virtual_simulation,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    config::{FileConfig, parse_seed},
    output::{TraceDigest, preflight, trace_digest, write_artifacts, write_trace_atomic},
};

const MANIFEST_FILE: &str = "manifest.json";
const CANONICAL_AGENTS: usize = 1_000;
const CANONICAL_DAYS: u16 = 30;
const MIN_CANONICAL_SEEDS: usize = 5;
const MAX_AGENT_DAYS: usize = 100_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SoakMode {
    Canonical,
    Quick,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct SoakManifest {
    schema_version: u16,
    mode: SoakMode,
    offline_only: bool,
    signer_loaded: bool,
    network_requests: u64,
    trace_hash_algorithm: String,
    scheduler: String,
    start: String,
    model_version: String,
    agents: usize,
    days: u16,
    max_concurrency: usize,
    max_decisions_per_agent_day: u32,
    daily_budget: u64,
    seeds: Vec<u64>,
    completed_seeds: usize,
    per_seed: Vec<SeedResult>,
    aggregate: Aggregate,
    proofs: Proofs,
    total_wall_time_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct SeedResult {
    seed: u64,
    seed_hex: String,
    run_id: String,
    decisions: u64,
    observable_events: u64,
    trace_bytes: u64,
    trace_blake3: String,
    trace_file: String,
    wall_time_ms: u64,
    replay_wall_time_ms: u64,
    proofs: Proofs,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Aggregate {
    decisions: RangeStats,
    observable_events: RangeStats,
    trace_bytes: RangeStats,
    wall_time_ms: RangeStats,
    replay_wall_time_ms: RangeStats,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct RangeStats {
    min: u64,
    mean: f64,
    max: u64,
}

#[allow(
    clippy::struct_excessive_bools,
    reason = "evidence manifests expose each independently checked invariant explicitly"
)]
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct Proofs {
    deterministic_replay: bool,
    chronological_events: bool,
    label_free_events: bool,
    bounded_scheduler_workers: bool,
    daily_budget_respected: bool,
    completed_without_panic: bool,
}

impl Proofs {
    const fn passed() -> Self {
        Self {
            deterministic_replay: true,
            chronological_events: true,
            label_free_events: true,
            bounded_scheduler_workers: true,
            daily_budget_respected: true,
            completed_without_panic: true,
        }
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "CLI arguments remain explicit at the command boundary"
)]
pub(crate) fn execute(
    config_path: &Path,
    output_dir: &Path,
    seeds: Option<Vec<u64>>,
    agents: Option<usize>,
    days: Option<u16>,
    quick: bool,
    verify_only: bool,
    force: bool,
    output: &mut impl Write,
) -> Result<()> {
    if verify_only {
        return verify_existing(output_dir, quick, output);
    }

    let file = FileConfig::load(config_path)?;
    file.validate()
        .with_context(|| format!("configuration {} is invalid", config_path.display()))?;
    let seeds = seeds.unwrap_or_else(|| file.evaluation.seeds.clone());
    let agents = agents.unwrap_or(file.offline.simulation_agents);
    let days = days.unwrap_or(file.offline.simulation_days);
    validate_workload(&seeds, agents, days, quick)?;

    let trace_paths = seeds
        .iter()
        .map(|seed| output_dir.join(trace_relative_path(*seed)))
        .collect::<Vec<_>>();
    let manifest_path = output_dir.join(MANIFEST_FILE);
    let mut all_paths = trace_paths.clone();
    all_paths.push(manifest_path.clone());
    preflight(&all_paths, force)?;

    let personas = file.offline.persona_configs();
    let catalog = file.offline.catalog(&file.core.enabled_actions);
    let total_started = Instant::now();
    let mut per_seed = Vec::with_capacity(seeds.len());

    for (seed, trace_path) in seeds.iter().copied().zip(trace_paths) {
        let seed_text = seed.to_string();
        let simulation =
            file.simulation(Some(agents), Some(days), Some(seed_text.as_str()), None)?;
        let result = run_seed(
            seed,
            &simulation,
            &personas,
            &catalog,
            output_dir,
            &trace_path,
            force,
        )?;
        writeln!(
            output,
            "soak seed {}: {} decisions, {} events, {} bytes, trace {}",
            result.seed,
            result.decisions,
            result.observable_events,
            result.trace_bytes,
            result.trace_blake3,
        )
        .context("failed to write soak progress")?;
        per_seed.push(result);
    }

    let sample = file.simulation(
        Some(agents),
        Some(days),
        Some(seeds[0].to_string().as_str()),
        None,
    )?;
    let manifest = SoakManifest {
        schema_version: 1,
        mode: if quick {
            SoakMode::Quick
        } else {
            SoakMode::Canonical
        },
        offline_only: true,
        signer_loaded: false,
        network_requests: 0,
        trace_hash_algorithm: "blake3".to_owned(),
        scheduler: "stable_bounded".to_owned(),
        start: sample.start.to_rfc3339(),
        model_version: sample.model_version,
        agents,
        days,
        max_concurrency: sample.max_concurrency,
        max_decisions_per_agent_day: sample.max_decisions_per_agent_day,
        daily_budget: sample.daily_budget,
        seeds: seeds.clone(),
        completed_seeds: per_seed.len(),
        aggregate: aggregate(&per_seed)?,
        per_seed,
        proofs: Proofs::passed(),
        total_wall_time_ms: elapsed_ms(total_started.elapsed()),
    };
    let bytes = pretty_json_line(&manifest)?;
    write_artifacts(vec![(manifest_path.clone(), bytes)], force)?;
    writeln!(
        output,
        "soak complete: {} seed(s), {} agent(s), {} day(s); manifest {}; no signer loaded and no network request made",
        seeds.len(),
        agents,
        days,
        manifest_path.display(),
    )
    .context("failed to write soak summary")
}

fn run_seed(
    seed: u64,
    simulation: &SimulationConfig,
    personas: &[cooker_core::PersonaConfig],
    catalog: &cooker_core::ActionCatalog,
    output_dir: &Path,
    trace_path: &Path,
    force: bool,
) -> Result<SeedResult> {
    let started = Instant::now();
    let artifact = run_virtual_simulation(simulation, personas, catalog)
        .map_err(anyhow::Error::msg)
        .with_context(|| format!("virtual soak seed {seed} failed"))?;
    validate_artifact(&artifact, simulation)?;
    let run_id = artifact.run_id.to_string();
    let decisions = artifact.decision_count;
    let observable_events =
        u64::try_from(artifact.events.len()).context("trace event count overflow")?;
    let written = write_trace_atomic(trace_path, &artifact.events, force)?;
    let wall_time_ms = elapsed_ms(started.elapsed());
    drop(artifact);

    let replay_started = Instant::now();
    let replay = run_virtual_simulation(simulation, personas, catalog)
        .map_err(anyhow::Error::msg)
        .with_context(|| format!("virtual soak replay for seed {seed} failed"))?;
    validate_artifact(&replay, simulation)?;
    let replay_digest = trace_digest(&replay.events)?;
    ensure!(
        replay.run_id.to_string() == run_id
            && replay.decision_count == decisions
            && u64::try_from(replay.events.len()).context("replay event count overflow")?
                == observable_events
            && replay_digest == written,
        "deterministic replay proof failed for seed {seed}"
    );
    let replay_wall_time_ms = elapsed_ms(replay_started.elapsed());
    drop(replay);

    let relative = trace_path
        .strip_prefix(output_dir)
        .with_context(|| format!("trace {} is outside soak output", trace_path.display()))?;
    Ok(SeedResult {
        seed,
        seed_hex: hex::encode(simulation.global_seed),
        run_id,
        decisions,
        observable_events,
        trace_bytes: written.bytes,
        trace_blake3: written.blake3,
        trace_file: path_to_manifest_string(relative)?,
        wall_time_ms,
        replay_wall_time_ms,
        proofs: Proofs::passed(),
    })
}

fn validate_artifact(artifact: &SimulationArtifact, simulation: &SimulationConfig) -> Result<()> {
    ensure!(
        artifact.agent_count == simulation.agent_count,
        "simulation reported an unexpected agent count"
    );
    ensure!(
        simulation.max_concurrency > 0 && simulation.max_concurrency <= simulation.agent_count,
        "scheduler worker bound is invalid"
    );
    let end = simulation
        .start
        .checked_add_days(Days::new(u64::from(simulation.days)))
        .context("simulation end overflow")?;
    let mut inspector = EventInspector::new(
        simulation.start,
        end,
        simulation.daily_budget,
        artifact.run_id.to_string(),
    );
    for event in &artifact.events {
        inspector.inspect(event, None)?;
    }
    Ok(())
}

struct EventInspector {
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    daily_budget: u64,
    run_id: String,
    previous: Option<(DateTime<Utc>, AgentId, u64)>,
    daily_spend: BTreeMap<(AgentId, NaiveDate), u64>,
}

impl EventInspector {
    fn new(start: DateTime<Utc>, end: DateTime<Utc>, daily_budget: u64, run_id: String) -> Self {
        Self {
            start,
            end,
            daily_budget,
            run_id,
            previous: None,
            daily_spend: BTreeMap::new(),
        }
    }

    fn inspect(&mut self, event: &TraceEvent, raw: Option<&Value>) -> Result<()> {
        ensure!(
            event.schema_version == TraceEvent::SCHEMA_VERSION,
            "trace contains an unsupported event schema"
        );
        ensure!(
            event.run_id.to_string() == self.run_id,
            "trace event run ID does not match its seed result"
        );
        ensure!(
            event.scheduled_at >= self.start && event.scheduled_at < self.end,
            "trace event is outside the virtual soak interval"
        );
        ensure!(
            event.observed_at.is_none()
                && event.signature.is_none()
                && event.outcome == TraceOutcome::Planned,
            "virtual trace contains online execution state"
        );
        ensure!(
            event.action_kind != ActionKind::Idle,
            "observable trace unexpectedly contains an idle decision"
        );
        ensure!(
            event.attributes.keys().all(|key| !is_label_key(key)),
            "trace attributes expose a hidden controller or persona label"
        );
        if let Some(value) = raw {
            ensure!(
                value_is_label_free(value),
                "trace row exposes a hidden controller or persona label"
            );
        }

        let ordering = (event.scheduled_at, event.agent_id, event.sequence);
        if let Some(previous) = self.previous {
            ensure!(
                ordering >= previous,
                "trace events are not in chronological stable order"
            );
        }
        self.previous = Some(ordering);

        let amount = match event.action_kind {
            ActionKind::NativeTransfer | ActionKind::JupiterSwap | ActionKind::StakeLifecycle => {
                event.amount
            }
            ActionKind::SplTransfer | ActionKind::Idle => 0,
        };
        let key = (event.agent_id, event.scheduled_at.date_naive());
        let spent = self.daily_spend.entry(key).or_default();
        *spent = spent
            .checked_add(amount)
            .context("daily trace spend overflow")?;
        ensure!(
            *spent <= self.daily_budget,
            "trace exceeds the configured per-agent daily budget"
        );
        Ok(())
    }
}

fn verify_existing(output_dir: &Path, quick: bool, output: &mut impl Write) -> Result<()> {
    let manifest_path = output_dir.join(MANIFEST_FILE);
    let manifest_bytes = fs::read(&manifest_path)
        .with_context(|| format!("failed to read soak manifest {}", manifest_path.display()))?;
    let manifest: SoakManifest = serde_json::from_slice(&manifest_bytes)
        .with_context(|| format!("failed to parse soak manifest {}", manifest_path.display()))?;
    verify_manifest_fields(&manifest, quick)?;

    let start = DateTime::parse_from_rfc3339(&manifest.start)
        .context("soak manifest start is not RFC3339")?
        .with_timezone(&Utc);
    let end = start
        .checked_add_days(Days::new(u64::from(manifest.days)))
        .context("soak manifest end overflow")?;
    let mut verified_results = Vec::with_capacity(manifest.per_seed.len());
    let mut trace_files = BTreeSet::new();

    for result in &manifest.per_seed {
        let relative = safe_relative_path(&result.trace_file)?;
        ensure!(
            trace_files.insert(relative.clone()),
            "soak manifest repeats trace file {}",
            result.trace_file
        );
        let scan = scan_trace(
            &output_dir.join(relative),
            start,
            end,
            manifest.daily_budget,
            &result.run_id,
        )?;
        ensure!(
            scan.bytes == result.trace_bytes
                && u64::try_from(scan.events).context("verified event count overflow")?
                    == result.observable_events
                && scan.blake3 == result.trace_blake3,
            "trace metadata or BLAKE3 digest mismatch for seed {}",
            result.seed
        );
        ensure!(
            result.decisions >= result.observable_events,
            "observable events exceed planner decisions for seed {}",
            result.seed
        );
        ensure!(
            result.seed_hex == hex::encode(parse_seed(&result.seed.to_string())?),
            "expanded seed mismatch for seed {}",
            result.seed
        );
        ensure!(
            result.proofs == Proofs::passed(),
            "seed {} does not report all required proofs",
            result.seed
        );
        verified_results.push(result.clone());
    }

    ensure!(
        aggregate(&verified_results)? == manifest.aggregate,
        "soak aggregate min/mean/max values do not match per-seed results"
    );
    let measured_ms = verified_results.iter().try_fold(0_u64, |total, result| {
        total
            .checked_add(result.wall_time_ms)
            .and_then(|value| value.checked_add(result.replay_wall_time_ms))
            .context("soak wall-time total overflow")
    })?;
    ensure!(
        manifest.total_wall_time_ms >= measured_ms,
        "soak total wall time is below summed seed wall times"
    );
    writeln!(
        output,
        "verified {} seed trace(s) and manifest {}; all hashes and invariants pass",
        manifest.completed_seeds,
        manifest_path.display(),
    )
    .context("failed to write soak verification summary")
}

fn verify_manifest_fields(manifest: &SoakManifest, quick: bool) -> Result<()> {
    ensure!(
        manifest.schema_version == 1,
        "unsupported soak manifest schema"
    );
    ensure!(
        manifest.offline_only && !manifest.signer_loaded && manifest.network_requests == 0,
        "soak manifest does not prove offline, signer-free execution"
    );
    ensure!(
        manifest.trace_hash_algorithm == "blake3",
        "soak manifest uses an unsupported trace hash"
    );
    ensure!(
        manifest.scheduler == "stable_bounded"
            && manifest.max_concurrency > 0
            && manifest.max_concurrency <= manifest.agents,
        "soak manifest scheduler worker bound is invalid"
    );
    ensure!(
        !manifest.model_version.trim().is_empty()
            && manifest.max_decisions_per_agent_day > 0
            && manifest.daily_budget > 0,
        "soak manifest simulation bounds are invalid"
    );
    validate_workload(
        &manifest.seeds,
        manifest.agents,
        manifest.days,
        manifest.mode == SoakMode::Quick,
    )?;
    if manifest.mode == SoakMode::Quick {
        ensure!(quick, "refusing quick soak evidence without --quick");
    }
    ensure!(
        manifest.completed_seeds == manifest.seeds.len()
            && manifest.per_seed.len() == manifest.seeds.len(),
        "soak manifest seed counts disagree"
    );
    ensure!(
        manifest
            .seeds
            .iter()
            .copied()
            .eq(manifest.per_seed.iter().map(|result| result.seed)),
        "soak manifest seed ordering disagrees with per-seed results"
    );
    ensure!(
        manifest.proofs == Proofs::passed(),
        "soak manifest does not report all required proofs"
    );
    Ok(())
}

fn scan_trace(
    path: &Path,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    daily_budget: u64,
    run_id: &str,
) -> Result<TraceDigest> {
    let file = File::open(path)
        .with_context(|| format!("failed to open soak trace {}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut inspector = EventInspector::new(start, end, daily_budget, run_id.to_owned());
    let mut hasher = blake3::Hasher::new();
    let mut bytes = 0_u64;
    let mut events = 0_usize;
    let mut row = Vec::new();

    loop {
        row.clear();
        let read = reader
            .read_until(b'\n', &mut row)
            .with_context(|| format!("failed to read soak trace {}", path.display()))?;
        if read == 0 {
            break;
        }
        ensure!(
            row.ends_with(b"\n"),
            "trace {} has a non-terminated JSONL row",
            path.display()
        );
        hasher.update(&row);
        bytes = bytes
            .checked_add(u64::try_from(read).context("trace byte count overflow")?)
            .context("trace byte count overflow")?;
        let raw: Value = serde_json::from_slice(&row)
            .with_context(|| format!("trace {} contains invalid JSON", path.display()))?;
        let event: TraceEvent = serde_json::from_value(raw.clone())
            .with_context(|| format!("trace {} contains an invalid event", path.display()))?;
        inspector.inspect(&event, Some(&raw))?;
        events = events
            .checked_add(1)
            .context("trace event count overflow")?;
    }
    Ok(TraceDigest {
        bytes,
        events,
        blake3: hasher.finalize().to_hex().to_string(),
    })
}

fn validate_workload(seeds: &[u64], agents: usize, days: u16, quick: bool) -> Result<()> {
    ensure!(
        !seeds.is_empty(),
        "soak requires at least one held-out seed"
    );
    let unique = seeds.iter().copied().collect::<BTreeSet<_>>();
    ensure!(
        unique.len() == seeds.len(),
        "soak seeds must be unique held-out values"
    );
    ensure!(
        agents > 0 && agents <= 10_000 && days > 0 && days <= 365,
        "soak requires agents in 1..=10000 and days in 1..=365"
    );
    let agent_days = agents
        .checked_mul(usize::from(days))
        .context("soak size overflow")?;
    ensure!(
        agent_days <= MAX_AGENT_DAYS,
        "soak is limited to {MAX_AGENT_DAYS} agent-days per seed"
    );
    if !quick {
        ensure!(
            seeds.len() >= MIN_CANONICAL_SEEDS,
            "canonical soak requires at least {MIN_CANONICAL_SEEDS} unique seeds; pass --quick for a reduced run"
        );
        ensure!(
            agents == CANONICAL_AGENTS && days == CANONICAL_DAYS,
            "canonical soak requires {CANONICAL_AGENTS} agents for {CANONICAL_DAYS} days; pass --quick for a reduced run"
        );
    }
    Ok(())
}

fn aggregate(results: &[SeedResult]) -> Result<Aggregate> {
    ensure!(!results.is_empty(), "cannot aggregate an empty soak");
    Ok(Aggregate {
        decisions: RangeStats::from_values(
            &results
                .iter()
                .map(|value| value.decisions)
                .collect::<Vec<_>>(),
        )?,
        observable_events: RangeStats::from_values(
            &results
                .iter()
                .map(|value| value.observable_events)
                .collect::<Vec<_>>(),
        )?,
        trace_bytes: RangeStats::from_values(
            &results
                .iter()
                .map(|value| value.trace_bytes)
                .collect::<Vec<_>>(),
        )?,
        wall_time_ms: RangeStats::from_values(
            &results
                .iter()
                .map(|value| value.wall_time_ms)
                .collect::<Vec<_>>(),
        )?,
        replay_wall_time_ms: RangeStats::from_values(
            &results
                .iter()
                .map(|value| value.replay_wall_time_ms)
                .collect::<Vec<_>>(),
        )?,
    })
}

impl RangeStats {
    #[allow(
        clippy::cast_precision_loss,
        reason = "manifest means are descriptive wall/count aggregates over bounded u64 samples"
    )]
    fn from_values(values: &[u64]) -> Result<Self> {
        let min = values.iter().copied().min().context("empty statistics")?;
        let max = values.iter().copied().max().context("empty statistics")?;
        let sum = values.iter().try_fold(0_u64, |total, value| {
            total.checked_add(*value).context("statistics sum overflow")
        })?;
        Ok(Self {
            min,
            mean: sum as f64 / values.len() as f64,
            max,
        })
    }
}

fn trace_relative_path(seed: u64) -> PathBuf {
    PathBuf::from("traces").join(format!("seed-{seed}.jsonl"))
}

fn safe_relative_path(value: &str) -> Result<PathBuf> {
    let path = PathBuf::from(value);
    ensure!(!path.as_os_str().is_empty(), "trace path is empty");
    ensure!(
        path.components()
            .all(|component| matches!(component, Component::Normal(_))),
        "trace path {value:?} is not a safe relative path"
    );
    Ok(path)
}

fn path_to_manifest_string(path: &Path) -> Result<String> {
    let value = path
        .to_str()
        .context("trace path is not valid UTF-8")?
        .replace(std::path::MAIN_SEPARATOR, "/");
    let _ = safe_relative_path(&value)?;
    Ok(value)
}

fn is_label_key(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase().replace('-', "_");
    normalized.contains("controller")
        || normalized.contains("persona")
        || normalized.contains("ground_truth")
        || normalized.contains("cohort")
}

fn value_is_label_free(value: &Value) -> bool {
    match value {
        Value::Object(object) => object
            .iter()
            .all(|(key, nested)| !is_label_key(key) && value_is_label_free(nested)),
        Value::Array(values) => values.iter().all(value_is_label_free),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => true,
    }
}

fn pretty_json_line(value: &impl Serialize) -> Result<Vec<u8>> {
    let mut bytes =
        serde_json::to_vec_pretty(value).context("failed to serialize soak manifest")?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn elapsed_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_bounds_require_five_unique_1000_by_30_seeds() {
        assert!(validate_workload(&[11, 23, 37, 51, 71], 1_000, 30, false).is_ok());
        assert!(validate_workload(&[11, 23, 37, 51], 1_000, 30, false).is_err());
        assert!(validate_workload(&[11, 11, 37, 51, 71], 1_000, 30, false).is_err());
        assert!(validate_workload(&[11, 23, 37, 51, 71], 10, 2, false).is_err());
        assert!(validate_workload(&[11], 10, 2, true).is_ok());
    }

    #[test]
    fn manifest_trace_paths_cannot_escape_output() {
        assert_eq!(
            safe_relative_path("traces/seed-11.jsonl").ok(),
            Some(PathBuf::from("traces/seed-11.jsonl"))
        );
        assert!(safe_relative_path("../seed-11.jsonl").is_err());
        assert!(safe_relative_path("/tmp/seed-11.jsonl").is_err());
    }
}
