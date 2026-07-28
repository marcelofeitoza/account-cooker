//! Offline command implementations and explicit online integration boundaries.

use std::{io::Write, path::PathBuf};

use anyhow::{Context, Result, bail};
use cooker_core::{
    ActionKind, AgentSnapshot, RunId, SessionState, TraceEvent, run_virtual_simulation,
};
use cooker_eval::{render_csv, render_markdown, run_experiment};
use cooker_solana::{FleetManifest, LocalKeypair, RpcEndpoint, SolanaGateway};
use cooker_store::{RunRegistration, Store, StoreIdentity, StoreStatusSnapshot};
use serde::Serialize;

use crate::{
    Command,
    config::{EXAMPLE_CONFIG, FileConfig},
    output::{preflight, write_artifacts},
};

#[derive(Debug, Serialize)]
struct SimulationSummary {
    schema_version: u16,
    run_id: String,
    start: String,
    days: u16,
    agents: usize,
    decisions: u64,
    observable_events: usize,
    trace_bytes: usize,
    trace_file: &'static str,
    trace_blake3: String,
    seed_hex: String,
}

#[derive(Debug, Serialize)]
struct DoctorReport {
    schema_version: u16,
    healthy: bool,
    surfnet_id: String,
    rpc_url: String,
    surfpool_version: String,
    solana_core: Option<String>,
    feature_set: Option<u64>,
    block_height: u64,
    epoch: u64,
    absolute_slot: u64,
    latest_blockhash: String,
    last_valid_block_height: u64,
    recent_local_signatures: usize,
    signer_loaded: bool,
    state_changed: bool,
}

#[derive(Debug, Serialize)]
struct StatusReport {
    schema_version: u16,
    database: String,
    store_schema_version: u32,
    journal_mode: String,
    integrity: &'static str,
    snapshot: StoreStatusSnapshot,
    network_requests: u8,
    signer_loaded: bool,
    state_changed: bool,
}

#[allow(clippy::too_many_lines)]
pub(crate) fn execute(command: Command, output: &mut impl Write) -> Result<()> {
    match command {
        Command::Keygen {
            project_root,
            output: path,
        } => keygen(&project_root, &path, output),
        Command::FleetInit {
            config,
            project_root,
            database,
            agents,
            start,
        } => fleet_init(
            &config,
            &project_root,
            &database,
            agents,
            start.as_deref(),
            output,
        ),
        Command::Fund {
            config,
            project_root,
            database,
            funder,
            scheme,
            recipient_project_roots,
            disbursers,
            lamports_per_agent,
            funding_rounds,
            top_ups_per_account,
            round_interval_seconds,
            limit,
            execute,
            acknowledge_policy,
        } => {
            let _ = acknowledge_policy;
            crate::online::fund(
                &config,
                &project_root,
                &database,
                &funder,
                scheme,
                &recipient_project_roots,
                &disbursers,
                lamports_per_agent,
                funding_rounds,
                top_ups_per_account,
                round_interval_seconds,
                limit,
                execute,
                output,
            )
        }
        Command::Init {
            output: path,
            force,
        } => init(&path, force, output),
        Command::Validate { config } => validate(&config, output),
        Command::Plan {
            config,
            limit,
            agents,
            days,
            seed,
        } => plan(&config, limit, agents, days, seed.as_deref(), output),
        Command::Simulate {
            config,
            output_dir,
            agents,
            days,
            seed,
            start,
            force,
        } => simulate(
            &config,
            &output_dir,
            agents,
            days,
            seed.as_deref(),
            start.as_deref(),
            force,
            output,
        ),
        Command::Evaluate {
            config,
            output_dir,
            force,
        } => evaluate(&config, &output_dir, force, output),
        Command::Soak {
            config,
            output_dir,
            seeds,
            agents,
            days,
            quick,
            verify_only,
            force,
        } => crate::soak::execute(
            &config,
            &output_dir,
            seeds,
            agents,
            days,
            quick,
            verify_only,
            force,
            output,
        ),
        Command::Doctor { config } => doctor(&config, output),
        Command::Run {
            config,
            project_root,
            database,
            limit,
            planning_limit,
            execute,
            acknowledge_policy,
        } => {
            let _ = acknowledge_policy;
            crate::online::run(
                &config,
                &project_root,
                &database,
                limit,
                planning_limit,
                execute,
                output,
            )
        }
        Command::Status {
            config,
            project_root,
            database,
        } => status(&config, &project_root, &database, output),
        Command::Recover {
            config,
            project_root,
            database,
            limit,
            audit_age_seconds,
            execute,
            acknowledge_policy,
        } => {
            let _ = acknowledge_policy;
            crate::online::recover(
                &config,
                &project_root,
                &database,
                limit,
                audit_age_seconds,
                execute,
                output,
            )
        }
    }
}

fn keygen(
    project_root: &std::path::Path,
    path: &std::path::Path,
    output: &mut impl Write,
) -> Result<()> {
    let root = project_root
        .canonicalize()
        .with_context(|| format!("invalid project root {}", project_root.display()))?;
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let signer = if resolved.exists() {
        LocalKeypair::load(&root, &resolved)
    } else {
        LocalKeypair::generate(&root, &resolved)
    }
    .map_err(anyhow::Error::msg)
    .context("failed to load or generate local Surfpool keypair")?;
    writeln!(
        output,
        "keypair {} {}",
        signer.pubkey(),
        signer.path().display()
    )
    .context("failed to write keygen output")
}

fn fleet_init(
    config_path: &std::path::Path,
    project_root: &std::path::Path,
    database: &std::path::Path,
    agents: usize,
    start: Option<&str>,
    output: &mut impl Write,
) -> Result<()> {
    if agents == 0 || agents > 10_000 {
        bail!("fleet-init requires agents in 1..=10000");
    }
    let file = load_validated(config_path)?;
    let root = project_root
        .canonicalize()
        .with_context(|| format!("invalid project root {}", project_root.display()))?;
    let created_at = chrono::Utc::now();
    let planner_start = start
        .map(|value| {
            chrono::DateTime::parse_from_rfc3339(value)
                .map(|timestamp| timestamp.with_timezone(&chrono::Utc))
                .with_context(|| format!("invalid fleet planner start {value:?}"))
        })
        .transpose()?
        .unwrap_or(created_at);
    if planner_start > created_at {
        bail!("fleet planner start cannot be in the future");
    }
    let run_id = execution_run_id(&file);
    let manifest = FleetManifest::create(
        &root,
        run_id,
        file.core.network.surfnet_id.clone(),
        agents,
        created_at,
    )
    .map_err(anyhow::Error::msg)
    .context("failed to create signer fleet")?;
    let database = if database.is_absolute() {
        database.to_path_buf()
    } else {
        root.join(database)
    };
    if let Some(parent) = database.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create database directory {}", parent.display()))?;
    }
    let store = Store::open(
        &database,
        StoreIdentity::surfpool(file.core.network.surfnet_id.clone())
            .map_err(anyhow::Error::msg)?,
    )
    .map_err(anyhow::Error::msg)
    .context("failed to initialize fleet store")?;
    let config_value = serde_json::to_value(&file).context("failed to encode run config")?;
    let config_bytes = serde_json::to_vec(&config_value).context("failed to hash run config")?;
    let registration = RunRegistration {
        id: run_id,
        model_version: "behavior-v1".to_owned(),
        config: config_value,
        config_hash: blake3::hash(&config_bytes).to_hex().to_string(),
        seed_hash: blake3::hash(file.core.seed_hex.as_bytes())
            .to_hex()
            .to_string(),
        created_at,
    };
    store
        .register_run(&registration)
        .map_err(anyhow::Error::msg)
        .context("failed to register fleet run")?;
    for entry in manifest.entries() {
        store
            .upsert_agent(
                &AgentSnapshot {
                    id: entry.agent_id,
                    run_id,
                    next_sequence: 0,
                    next_decision_at: planner_start,
                    budget_date: planner_start.date_naive(),
                    session_state: SessionState::Dormant,
                    last_action_at: None,
                    remaining_daily_budget: file.core.budgets.daily_lamports,
                    model_version: "behavior-v1".to_owned(),
                },
                created_at,
            )
            .map_err(anyhow::Error::msg)
            .with_context(|| format!("failed to initialize agent {}", entry.agent_id))?;
    }
    writeln!(
        output,
        "initialized fleet {}: {} unique signer(s), run {}, database {}",
        manifest.surfnet_id(),
        manifest.entries().len(),
        manifest.run_id(),
        database.display(),
    )
    .context("failed to write fleet summary")
}

pub(crate) fn execution_run_id(file: &FileConfig) -> RunId {
    const NAMESPACE: uuid::Uuid = uuid::Uuid::from_u128(0xa9cf_21f4_7b0a_58ca_84bd_dfb8_b03b_1191);
    let mut identity = Vec::new();
    identity.extend_from_slice(file.core.seed_hex.as_bytes());
    identity.extend_from_slice(file.core.network.surfnet_id.as_bytes());
    identity.extend_from_slice(b"behavior-v1");
    RunId(uuid::Uuid::new_v5(&NAMESPACE, &identity))
}

fn init(path: &std::path::Path, force: bool, output: &mut impl Write) -> Result<()> {
    let config: FileConfig =
        toml::from_str(EXAMPLE_CONFIG).context("embedded example configuration failed to parse")?;
    config
        .validate()
        .context("embedded example configuration failed validation")?;
    write_artifacts(
        vec![(path.to_path_buf(), EXAMPLE_CONFIG.as_bytes().to_vec())],
        force,
    )?;
    writeln!(output, "initialized {}", path.display()).context("failed to write command output")
}

fn validate(path: &std::path::Path, output: &mut impl Write) -> Result<()> {
    let config = load_validated(path)?;
    writeln!(
        output,
        "configuration valid: {} action(s), {} persona(s), {} evaluation seed(s); no network request made",
        config.core.enabled_actions.len(),
        config.offline.personas.len(),
        config.evaluation.seeds.len(),
    )
    .context("failed to write command output")
}

fn doctor(path: &std::path::Path, output: &mut impl Write) -> Result<()> {
    let config = load_validated(path)?;
    let endpoint = RpcEndpoint::new(config.core.network.rpc_url.clone())
        .map_err(anyhow::Error::msg)
        .context("RPC endpoint is not a strict explicit-loopback Surfpool URL")?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to create doctor async runtime")?;
    let report = runtime.block_on(async {
        let gateway = SolanaGateway::connect(endpoint)
            .await
            .map_err(anyhow::Error::msg)
            .context("Surfpool identity connection failed")?;
        let identity = gateway
            .doctor()
            .await
            .map_err(anyhow::Error::msg)
            .context("repeated Surfpool identity probe failed")?;
        let (block_height, epoch, blockhash, local_signatures) = tokio::try_join!(
            gateway.block_height(),
            gateway.epoch_info(),
            gateway.latest_blockhash(),
            gateway.local_signatures(100),
        )
        .map_err(anyhow::Error::msg)
        .context("Surfpool health reads failed")?;
        Ok::<DoctorReport, anyhow::Error>(DoctorReport {
            schema_version: 1,
            healthy: true,
            surfnet_id: config.core.network.surfnet_id.clone(),
            rpc_url: gateway.endpoint().to_string(),
            surfpool_version: identity.surfnet_version,
            solana_core: identity.solana_core,
            feature_set: identity.feature_set,
            block_height,
            epoch: epoch.epoch,
            absolute_slot: epoch.absolute_slot,
            latest_blockhash: blockhash.blockhash.to_string(),
            last_valid_block_height: blockhash.last_valid_block_height,
            recent_local_signatures: local_signatures.len(),
            signer_loaded: false,
            state_changed: false,
        })
    })?;
    serde_json::to_writer_pretty(&mut *output, &report)
        .context("failed to write doctor JSON report")?;
    writeln!(output).context("failed to terminate doctor JSON report")
}

fn status(
    config_path: &std::path::Path,
    project_root: &std::path::Path,
    database: &std::path::Path,
    output: &mut impl Write,
) -> Result<()> {
    let file = load_validated(config_path)?;
    let database = resolve_project_path(project_root, database)?;
    if !database.is_file() {
        bail!("runtime database does not exist: {}", database.display());
    }
    let store = Store::open_read_only(
        &database,
        StoreIdentity::surfpool(file.core.network.surfnet_id.clone())
            .map_err(anyhow::Error::msg)?,
    )
    .map_err(anyhow::Error::msg)
    .context("failed to open runtime store")?;
    store
        .verify_integrity()
        .map_err(anyhow::Error::msg)
        .context("runtime store integrity check failed")?;
    let snapshot = store
        .status_snapshot(chrono::Utc::now(), None)
        .map_err(anyhow::Error::msg)
        .context("failed to capture runtime status")?;
    let report = StatusReport {
        schema_version: 1,
        database: database.display().to_string(),
        store_schema_version: store.schema_version().map_err(anyhow::Error::msg)?,
        journal_mode: "wal".to_owned(),
        integrity: "ok",
        snapshot,
        network_requests: 0,
        signer_loaded: false,
        state_changed: false,
    };
    serde_json::to_writer_pretty(&mut *output, &report)
        .context("failed to write status JSON report")?;
    writeln!(output).context("failed to terminate status JSON report")
}

fn resolve_project_path(project_root: &std::path::Path, path: &std::path::Path) -> Result<PathBuf> {
    let root = project_root
        .canonicalize()
        .with_context(|| format!("invalid project root {}", project_root.display()))?;
    Ok(if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    })
}

fn plan(
    path: &std::path::Path,
    limit: usize,
    agents: usize,
    days: u16,
    seed: Option<&str>,
    output: &mut impl Write,
) -> Result<()> {
    if limit == 0 || limit > 1_000 || agents == 0 || agents > 100 || days == 0 || days > 30 {
        bail!("plan requires limit in 1..=1000, agents in 1..=100, and days in 1..=30");
    }
    let file = load_validated(path)?;
    let simulation = file.simulation(Some(agents), Some(days), seed, None)?;
    let artifact = run_virtual_simulation(
        &simulation,
        &file.offline.persona_configs(),
        &file.offline.catalog(&file.core.enabled_actions),
    )
    .map_err(anyhow::Error::msg)
    .context("failed to build plan preview")?;
    writeln!(output, "# scheduled_at\taction\tamount\tdestination")
        .context("failed to write plan header")?;
    for event in artifact.events.iter().take(limit) {
        write_plan_row(output, event)?;
    }
    writeln!(
        output,
        "# showing {} of {} observable action(s); {} planner decisions; no signer loaded and no network request made",
        limit.min(artifact.events.len()),
        artifact.events.len(),
        artifact.decision_count,
    )
    .context("failed to write plan summary")
}

#[allow(clippy::too_many_arguments)]
fn simulate(
    path: &std::path::Path,
    output_dir: &std::path::Path,
    agents: Option<usize>,
    days: Option<u16>,
    seed: Option<&str>,
    start: Option<&str>,
    force: bool,
    output: &mut impl Write,
) -> Result<()> {
    if agents.is_some_and(|value| value == 0 || value > 10_000)
        || days.is_some_and(|value| value == 0 || value > 365)
    {
        bail!("simulate requires agents in 1..=10000 and days in 1..=365");
    }
    let trace_path = output_dir.join("trace.jsonl");
    let summary_path = output_dir.join("summary.json");
    preflight(&[trace_path.clone(), summary_path.clone()], force)?;
    let file = load_validated(path)?;
    let simulation = file.simulation(agents, days, seed, start)?;
    let agent_days = simulation
        .agent_count
        .checked_mul(usize::from(simulation.days))
        .context("simulation size overflow")?;
    if agent_days > 100_000 {
        bail!("simulate is limited to 100000 agent-days");
    }
    let artifact = run_virtual_simulation(
        &simulation,
        &file.offline.persona_configs(),
        &file.offline.catalog(&file.core.enabled_actions),
    )
    .map_err(anyhow::Error::msg)
    .context("simulation failed")?;
    let trace = artifact
        .trace_jsonl()
        .map_err(anyhow::Error::msg)
        .context("failed to serialize trace JSONL")?;
    let trace_hash = blake3::hash(&trace).to_hex().to_string();
    let summary = SimulationSummary {
        schema_version: 1,
        run_id: artifact.run_id.to_string(),
        start: simulation.start.to_rfc3339(),
        days: simulation.days,
        agents: simulation.agent_count,
        decisions: artifact.decision_count,
        observable_events: artifact.events.len(),
        trace_bytes: trace.len(),
        trace_file: "trace.jsonl",
        trace_blake3: trace_hash.clone(),
        seed_hex: hex::encode(simulation.global_seed),
    };
    let summary_bytes = pretty_json_line(&summary)?;
    write_artifacts(
        vec![(trace_path, trace), (summary_path, summary_bytes)],
        force,
    )?;
    writeln!(
        output,
        "simulated {} agent(s) for {} day(s): {} decisions, {} observable events, trace {}",
        simulation.agent_count,
        simulation.days,
        artifact.decision_count,
        artifact.events.len(),
        trace_hash,
    )
    .context("failed to write simulation summary")
}

fn evaluate(
    path: &std::path::Path,
    output_dir: &std::path::Path,
    force: bool,
    output: &mut impl Write,
) -> Result<()> {
    let json_path = output_dir.join("evaluation.json");
    let csv_path = output_dir.join("evaluation.csv");
    let markdown_path = output_dir.join("evaluation.md");
    preflight(
        &[json_path.clone(), csv_path.clone(), markdown_path.clone()],
        force,
    )?;
    let file = load_validated(path)?;
    let experiment = file.evaluation.experiment();
    let result = run_experiment(experiment)
        .map_err(anyhow::Error::msg)
        .context("evaluation experiment failed")?;
    let json = pretty_json_line(&result)?;
    let csv = render_csv(&result).into_bytes();
    let markdown = render_markdown(&result).into_bytes();
    write_artifacts(
        vec![
            (json_path, json),
            (csv_path, csv),
            (markdown_path, markdown),
        ],
        force,
    )?;
    writeln!(
        output,
        "evaluated {} held-out seed(s) across naive_uniform, independent_weighted, and persona_session: {} per-seed/ablation rows",
        file.evaluation.seeds.len(),
        result.seeds.len(),
    )
    .context("failed to write evaluation summary")
}

fn load_validated(path: &std::path::Path) -> Result<FileConfig> {
    let config = FileConfig::load(path)?;
    config
        .validate()
        .with_context(|| format!("configuration {} is invalid", path.display()))?;
    Ok(config)
}

fn write_plan_row(output: &mut impl Write, event: &TraceEvent) -> Result<()> {
    let destination = event.destination.as_deref().unwrap_or("-");
    writeln!(
        output,
        "{}\t{}\t{}\t{}",
        event.scheduled_at.to_rfc3339(),
        action_name(event.action_kind),
        event.amount,
        destination,
    )
    .context("failed to write plan row")
}

const fn action_name(kind: ActionKind) -> &'static str {
    match kind {
        ActionKind::NativeTransfer => "native_transfer",
        ActionKind::SplTransfer => "spl_transfer",
        ActionKind::JupiterSwap => "jupiter_swap",
        ActionKind::StakeLifecycle => "stake_lifecycle",
        ActionKind::Idle => "idle",
    }
}

fn pretty_json_line(value: &impl Serialize) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec_pretty(value).context("failed to serialize JSON")?;
    bytes.push(b'\n');
    Ok(bytes)
}
