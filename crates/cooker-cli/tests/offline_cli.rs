//! End-to-end tests for offline CLI parsing, output, and failure semantics.

#![forbid(unsafe_code)]

use std::{fs, path::Path};

use account_cooker::{Cli, execute};
use anyhow::{Context, Result};
use clap::{CommandFactory, Parser};
use cooker_solana::FleetManifest;
use cooker_store::{Store, StoreIdentity};
use serde_json::Value;
use tempfile::tempdir;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

fn invoke(arguments: &[&str]) -> Result<String> {
    let cli = Cli::try_parse_from(arguments)?;
    let mut output = Vec::new();
    execute(cli, &mut output)?;
    String::from_utf8(output).context("CLI output was not UTF-8")
}

fn initialize(path: &Path) -> Result<String> {
    invoke(&["cooker", "init", "--output", &path.to_string_lossy()])
}

#[test]
#[allow(clippy::too_many_lines)]
fn keygen_and_fleet_init_create_private_durable_unique_signers() -> Result<()> {
    let directory = tempdir()?;
    let root = directory.path();
    let config = root.join("cooker.toml");
    initialize(&config)?;

    let key_output = invoke(&[
        "cooker",
        "keygen",
        "--project-root",
        &root.to_string_lossy(),
        "--output",
        ".surfpool/keys/operator.json",
    ])?;
    assert!(key_output.starts_with("keypair "));
    let operator = root.join(".surfpool/keys/operator.json");
    let first_key = fs::read(&operator)?;
    assert_eq!(
        invoke(&[
            "cooker",
            "keygen",
            "--project-root",
            &root.to_string_lossy(),
            "--output",
            ".surfpool/keys/operator.json",
        ])?,
        key_output,
        "loading an existing safe key must be idempotent"
    );
    assert_eq!(fs::read(&operator)?, first_key);
    #[cfg(unix)]
    assert_eq!(fs::metadata(&operator)?.permissions().mode() & 0o777, 0o600);

    let database = ".surfpool/state/fleet.sqlite";
    let fleet_output = invoke(&[
        "cooker",
        "fleet-init",
        "--config",
        &config.to_string_lossy(),
        "--project-root",
        &root.to_string_lossy(),
        "--database",
        database,
        "--agents",
        "3",
    ])?;
    assert!(fleet_output.contains("3 unique signer(s)"));
    let manifest = FleetManifest::load(root).map_err(anyhow::Error::msg)?;
    assert_eq!(manifest.entries().len(), 3);
    let signers = manifest.load_signers(root).map_err(anyhow::Error::msg)?;
    assert_eq!(signers.len(), 3);
    let store = Store::open(
        root.join(database),
        StoreIdentity::surfpool("noise-dev").map_err(anyhow::Error::msg)?,
    )
    .map_err(anyhow::Error::msg)?;
    for entry in manifest.entries() {
        let snapshot = store
            .get_agent(entry.agent_id)
            .map_err(anyhow::Error::msg)?;
        assert_eq!(snapshot.run_id, manifest.run_id());
        assert_eq!(snapshot.next_sequence, 0);
    }

    let database_path = root.join(database);
    let sidecars = [
        database_path.clone(),
        Path::new(&format!("{}-wal", database_path.display())).to_path_buf(),
        Path::new(&format!("{}-shm", database_path.display())).to_path_buf(),
    ];
    let before: Vec<_> = sidecars
        .iter()
        .filter(|path| path.exists())
        .map(|path| Ok((path.clone(), fs::read(path)?)))
        .collect::<Result<_>>()?;

    for command in ["run", "recover"] {
        let report: Value = serde_json::from_str(&invoke(&[
            "cooker",
            command,
            "--config",
            &config.to_string_lossy(),
            "--project-root",
            &root.to_string_lossy(),
            "--database",
            database,
        ])?)?;
        assert_eq!(report["mode"], "preview");
        assert_eq!(report["network_preflight"], false);
        assert_eq!(report["signer_loaded"], false);
        assert_eq!(report["state_changed"], false);
    }
    let status: Value = serde_json::from_str(&invoke(&[
        "cooker",
        "status",
        "--config",
        &config.to_string_lossy(),
        "--project-root",
        &root.to_string_lossy(),
        "--database",
        database,
    ])?)?;
    assert_eq!(status["journal_mode"], "wal");
    assert_eq!(status["network_requests"], 0);
    assert_eq!(status["signer_loaded"], false);
    assert_eq!(status["state_changed"], false);
    let funding: Value = serde_json::from_str(&invoke(&[
        "cooker",
        "fund",
        "--config",
        &config.to_string_lossy(),
        "--project-root",
        &root.to_string_lossy(),
    ])?)?;
    assert_eq!(funding["mode"], "preview");
    assert_eq!(funding["network_preflight"], false);
    assert_eq!(funding["signer_loaded"], false);
    assert_eq!(funding["state_changed"], false);
    assert!(!root.join(".surfpool/state/funding.sqlite").exists());
    for (path, bytes) in before {
        assert_eq!(fs::read(path)?, bytes, "preview changed a store file");
    }

    let repeat = Cli::try_parse_from([
        "cooker",
        "fleet-init",
        "--config",
        &config.to_string_lossy(),
        "--project-root",
        &root.to_string_lossy(),
        "--database",
        database,
        "--agents",
        "3",
    ])?;
    assert!(execute(repeat, &mut Vec::new()).is_err());
    Ok(())
}

#[test]
fn clap_command_tree_and_future_safety_flags_are_valid() {
    Cli::command().debug_assert();
    assert!(Cli::try_parse_from(["cooker", "validate"]).is_ok());
    assert!(Cli::try_parse_from(["cooker", "plan", "--limit", "3"]).is_ok());
    assert!(Cli::try_parse_from(["cooker", "simulate", "--agents", "1000"]).is_ok());
    assert!(Cli::try_parse_from(["cooker", "evaluate"]).is_ok());
    assert!(
        Cli::try_parse_from(["cooker", "soak", "--quick", "--seeds", "11,23,37,51,71"]).is_ok()
    );
    assert!(Cli::try_parse_from(["cooker", "doctor"]).is_ok());
    assert!(Cli::try_parse_from(["cooker", "fund", "--execute", "--acknowledge-policy"]).is_ok());
    assert!(Cli::try_parse_from(["cooker", "run", "--execute", "--acknowledge-policy"]).is_ok());
    assert!(Cli::try_parse_from(["cooker", "status"]).is_ok());
    assert!(
        Cli::try_parse_from(["cooker", "recover", "--execute", "--acknowledge-policy"]).is_ok()
    );
    assert!(Cli::try_parse_from(["cooker", "recover", "--execute"]).is_err());
    assert!(
        Cli::try_parse_from(["cooker", "run", "--acknowledge-policy"]).is_err(),
        "policy acknowledgement without execution must be rejected"
    );
    assert!(
        Cli::try_parse_from(["cooker", "run", "--execute"]).is_err(),
        "execution without policy acknowledgement must be rejected"
    );
    assert!(Cli::try_parse_from(["cooker", "fund", "--execute"]).is_err());
    assert!(Cli::try_parse_from(["cooker", "fund", "--acknowledge-policy"]).is_err());
}

#[test]
fn init_is_valid_and_refuses_overwrite_without_force() -> Result<()> {
    let directory = tempdir()?;
    let path = directory.path().join("cooker.toml");
    assert!(initialize(&path)?.contains("initialized"));
    let original = fs::read(&path)?;
    let second = Cli::try_parse_from(["cooker", "init", "--output", &path.to_string_lossy()])?;
    assert!(execute(second, &mut Vec::new()).is_err());
    assert_eq!(fs::read(&path)?, original);

    let forced = invoke(&[
        "cooker",
        "init",
        "--output",
        &path.to_string_lossy(),
        "--force",
    ])?;
    assert!(forced.contains("initialized"));
    assert_eq!(fs::read(&path)?, original);
    Ok(())
}

#[test]
fn validate_and_plan_are_offline_and_bounded() -> Result<()> {
    let directory = tempdir()?;
    let config = directory.path().join("cooker.toml");
    initialize(&config)?;

    let validation = invoke(&["cooker", "validate", "--config", &config.to_string_lossy()])?;
    assert!(validation.contains("configuration valid"));
    assert!(validation.contains("no network request made"));

    let preview = invoke(&[
        "cooker",
        "plan",
        "--config",
        &config.to_string_lossy(),
        "--limit",
        "5",
        "--agents",
        "8",
        "--days",
        "7",
        "--seed",
        "42",
    ])?;
    assert!(preview.starts_with("# scheduled_at"));
    assert!(preview.contains("no signer loaded and no network request made"));
    assert!(preview.lines().count() <= 7, "header + five rows + summary");
    Ok(())
}

#[test]
fn simulation_writes_stable_jsonl_and_matching_summary() -> Result<()> {
    let directory = tempdir()?;
    let config = directory.path().join("cooker.toml");
    initialize(&config)?;
    let first = directory.path().join("first");
    let second = directory.path().join("second");

    for output in [&first, &second] {
        let summary = invoke(&[
            "cooker",
            "simulate",
            "--config",
            &config.to_string_lossy(),
            "--output-dir",
            &output.to_string_lossy(),
            "--agents",
            "12",
            "--days",
            "2",
            "--seed",
            "42",
        ])?;
        assert!(summary.contains("simulated 12 agent(s) for 2 day(s)"));
    }

    let first_trace = fs::read(first.join("trace.jsonl"))?;
    let second_trace = fs::read(second.join("trace.jsonl"))?;
    assert_eq!(first_trace, second_trace);
    assert!(!first_trace.is_empty());
    assert_eq!(
        fs::read(first.join("summary.json"))?,
        fs::read(second.join("summary.json"))?
    );

    let summary: Value = serde_json::from_slice(&fs::read(first.join("summary.json"))?)?;
    assert_eq!(summary["agents"], 12);
    assert_eq!(summary["days"], 2);
    assert_eq!(summary["trace_file"], "trace.jsonl");
    let expected_hash = blake3::hash(&first_trace).to_hex().to_string();
    assert_eq!(summary["trace_blake3"], expected_hash);
    assert!(first_trace.ends_with(b"\n"));

    let repeat = Cli::try_parse_from([
        "cooker",
        "simulate",
        "--config",
        &config.to_string_lossy(),
        "--output-dir",
        &first.to_string_lossy(),
        "--agents",
        "12",
        "--days",
        "2",
    ])?;
    assert!(execute(repeat, &mut Vec::new()).is_err());
    Ok(())
}

#[test]
fn evaluation_writes_json_csv_and_markdown_atomically() -> Result<()> {
    let directory = tempdir()?;
    let config = directory.path().join("cooker.toml");
    initialize(&config)?;
    let small = fs::read_to_string(&config)?
        .replace("controllers = 10", "controllers = 2")
        .replace("agents_per_controller = 10", "agents_per_controller = 2")
        .replace("days = 30", "days = 1")
        .replace(
            "events_per_agent_per_day = 2",
            "events_per_agent_per_day = 1",
        );
    fs::write(&config, small)?;
    let output = directory.path().join("evaluation");

    let status = invoke(&[
        "cooker",
        "evaluate",
        "--config",
        &config.to_string_lossy(),
        "--output-dir",
        &output.to_string_lossy(),
    ])?;
    assert!(status.contains("5 held-out seed(s)"));
    assert!(status.contains("naive_uniform, independent_weighted, and persona_session"));

    let json: Value = serde_json::from_slice(&fs::read(output.join("evaluation.json"))?)?;
    assert_eq!(json["config"]["seeds"].as_array().map(Vec::len), Some(5));
    let csv = fs::read_to_string(output.join("evaluation.csv"))?;
    assert!(csv.starts_with("funding_scheme,planner,seed,ablation"));
    let markdown = fs::read_to_string(output.join("evaluation.md"))?;
    assert!(markdown.contains("do not establish transaction-graph anonymity"));
    assert!(markdown.contains("## Funding Provenance"));
    assert!(markdown.contains("Funding batch AUC mean"));
    assert_eq!(
        json["config"]["funding_schemes"].as_array().map(Vec::len),
        Some(3)
    );

    let repeat = Cli::try_parse_from([
        "cooker",
        "evaluate",
        "--config",
        &config.to_string_lossy(),
        "--output-dir",
        &output.to_string_lossy(),
    ])?;
    assert!(execute(repeat, &mut Vec::new()).is_err());
    Ok(())
}

#[test]
fn quick_soak_replays_five_seeds_and_verifies_streamed_evidence() -> Result<()> {
    let directory = tempdir()?;
    let config = directory.path().join("cooker.toml");
    initialize(&config)?;
    let output = directory.path().join("virtual-soak");

    let status = invoke(&[
        "cooker",
        "soak",
        "--config",
        &config.to_string_lossy(),
        "--output-dir",
        &output.to_string_lossy(),
        "--seeds",
        "11,23,37,51,71",
        "--agents",
        "8",
        "--days",
        "2",
        "--quick",
    ])?;
    assert!(status.contains("soak complete: 5 seed(s), 8 agent(s), 2 day(s)"));
    assert!(status.contains("no signer loaded and no network request made"));

    let manifest: Value = serde_json::from_slice(&fs::read(output.join("manifest.json"))?)?;
    assert_eq!(manifest["schema_version"], 1);
    assert_eq!(manifest["mode"], "quick");
    assert_eq!(manifest["agents"], 8);
    assert_eq!(manifest["days"], 2);
    assert_eq!(manifest["completed_seeds"], 5);
    assert_eq!(manifest["network_requests"], 0);
    assert_eq!(manifest["signer_loaded"], false);
    assert_eq!(manifest["proofs"]["deterministic_replay"], true);
    assert_eq!(manifest["proofs"]["chronological_events"], true);
    assert_eq!(manifest["proofs"]["label_free_events"], true);
    assert_eq!(manifest["proofs"]["bounded_scheduler_workers"], true);
    assert_eq!(manifest["proofs"]["daily_budget_respected"], true);
    assert_eq!(manifest["proofs"]["completed_without_panic"], true);
    assert_eq!(manifest["per_seed"].as_array().map(Vec::len), Some(5));
    for result in manifest["per_seed"]
        .as_array()
        .context("per_seed was not an array")?
    {
        let trace_file = result["trace_file"]
            .as_str()
            .context("trace_file was not a string")?;
        let trace = fs::read(output.join(trace_file))?;
        assert!(trace.ends_with(b"\n"));
        assert_eq!(
            result["trace_blake3"],
            blake3::hash(&trace).to_hex().to_string()
        );
    }

    let verified = invoke(&[
        "cooker",
        "soak",
        "--output-dir",
        &output.to_string_lossy(),
        "--verify-only",
        "--quick",
    ])?;
    assert!(verified.contains("verified 5 seed trace(s)"));
    assert!(verified.contains("all hashes and invariants pass"));

    fs::write(output.join("traces/seed-11.jsonl"), b"{}\n")?;
    let corrupted = Cli::try_parse_from([
        "cooker",
        "soak",
        "--output-dir",
        &output.to_string_lossy(),
        "--verify-only",
        "--quick",
    ])?;
    assert!(execute(corrupted, &mut Vec::new()).is_err());
    Ok(())
}

#[test]
fn canonical_soak_rejects_reduced_workloads_before_simulation() -> Result<()> {
    let directory = tempdir()?;
    let config = directory.path().join("cooker.toml");
    initialize(&config)?;

    for arguments in [
        vec![
            "cooker",
            "soak",
            "--config",
            config.to_str().context("config path was not UTF-8")?,
            "--agents",
            "8",
            "--days",
            "2",
        ],
        vec![
            "cooker",
            "soak",
            "--config",
            config.to_str().context("config path was not UTF-8")?,
            "--seeds",
            "11,23,37,51",
        ],
    ] {
        let command = Cli::try_parse_from(arguments)?;
        assert!(execute(command, &mut Vec::new()).is_err());
    }
    Ok(())
}

#[test]
fn online_commands_fail_closed_when_required_files_are_absent() -> Result<()> {
    for arguments in [
        vec!["cooker", "doctor"],
        vec!["cooker", "run"],
        vec!["cooker", "status"],
        vec!["cooker", "recover"],
    ] {
        let cli = Cli::try_parse_from(arguments)?;
        let result = execute(cli, &mut Vec::new());
        assert!(result.is_err(), "reserved command must fail");
        let error = result
            .err()
            .context("reserved command unexpectedly succeeded")?;
        let message = format!("{error:#}");
        assert!(message.contains("configuration") || message.contains("cooker.toml"));
    }
    Ok(())
}

#[test]
fn filesystem_kill_switch_blocks_all_mutating_commands_before_preflight() -> Result<()> {
    let directory = tempdir()?;
    let root = directory.path();
    let config = root.join("cooker.toml");
    initialize(&config)?;
    let switch = root.join(".surfpool/KILL_SWITCH");
    fs::create_dir_all(switch.parent().context("kill-switch parent is missing")?)?;
    fs::write(&switch, b"stop\n")?;

    for command in ["fund", "run", "recover"] {
        let cli = Cli::try_parse_from([
            "cooker",
            command,
            "--config",
            &config.to_string_lossy(),
            "--project-root",
            &root.to_string_lossy(),
            "--execute",
            "--acknowledge-policy",
        ])?;
        let error = execute(cli, &mut Vec::new())
            .err()
            .with_context(|| format!("{command} ignored the filesystem kill switch"))?;
        assert!(format!("{error:#}").contains("filesystem kill switch"));
    }
    assert!(!root.join(".surfpool/state/funding.sqlite").exists());
    assert!(!root.join(".surfpool/state/cooker.sqlite").exists());
    Ok(())
}

#[test]
fn malformed_seed_and_out_of_range_plan_are_errors() -> Result<()> {
    let directory = tempdir()?;
    let config = directory.path().join("cooker.toml");
    initialize(&config)?;
    for arguments in [
        vec![
            "cooker".to_owned(),
            "plan".to_owned(),
            "--config".to_owned(),
            config.to_string_lossy().into_owned(),
            "--seed".to_owned(),
            "bad-seed".to_owned(),
        ],
        vec![
            "cooker".to_owned(),
            "plan".to_owned(),
            "--config".to_owned(),
            config.to_string_lossy().into_owned(),
            "--limit".to_owned(),
            "0".to_owned(),
        ],
    ] {
        let cli = Cli::try_parse_from(arguments)?;
        assert!(execute(cli, &mut Vec::new()).is_err());
    }
    Ok(())
}
