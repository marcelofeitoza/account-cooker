//! Offline-first command-line interface for deterministic planning and evaluation.

#![forbid(unsafe_code)]
#![allow(
    clippy::missing_errors_doc,
    reason = "all CLI failures carry command and path context through anyhow"
)]
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        reason = "test fixtures unwrap only values whose validity is part of setup"
    )
)]

mod commands;
mod config;
mod online;
mod output;
mod soak;

use std::{io::Write, path::PathBuf};

use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

/// Account Cooker command-line arguments.
#[derive(Clone, Debug, Parser)]
#[command(name = "cooker", version, about)]
pub struct Cli {
    /// Selected operation.
    #[command(subcommand)]
    pub command: Command,
}

/// Supported offline and Surfpool-only commands.
#[derive(Clone, Debug, Subcommand)]
pub enum Command {
    /// Generate one non-overwriting, permission-checked local Surfpool keypair.
    Keygen {
        /// Project root containing the ignored `.surfpool/keys` directory.
        #[arg(long, default_value = ".")]
        project_root: PathBuf,
        /// Destination under `.surfpool/keys`.
        #[arg(short, long, default_value = ".surfpool/keys/funder.json")]
        output: PathBuf,
    },
    /// Create a durable multi-signer fleet and initialize its `SQLite` planner cursors.
    FleetInit {
        /// Configuration path.
        #[arg(short, long, default_value = "cooker.toml")]
        config: PathBuf,
        /// Project root receiving the ignored manifest and signer files.
        #[arg(long, default_value = ".")]
        project_root: PathBuf,
        /// Runtime `SQLite` database.
        #[arg(long, default_value = ".surfpool/state/cooker.sqlite")]
        database: PathBuf,
        /// Number of independently signed persistent agents.
        #[arg(long, default_value_t = 1_000)]
        agents: usize,
        /// Optional RFC3339 planner cursor start; must not be in the future.
        #[arg(long)]
        start: Option<String>,
    },
    /// Preview or execute durable local funding for the generated fleet.
    Fund {
        /// Configuration path.
        #[arg(short, long, default_value = "cooker.toml")]
        config: PathBuf,
        /// Project root containing the ignored fleet and funder key files.
        #[arg(long, default_value = ".")]
        project_root: PathBuf,
        /// Isolated durable funding database.
        #[arg(long, default_value = ".surfpool/state/funding.sqlite")]
        database: PathBuf,
        /// Funder key under the ignored `.surfpool/keys` directory.
        #[arg(long, default_value = ".surfpool/keys/funder.json")]
        funder: PathBuf,
        /// Exact native amount sent once to each fleet signer.
        #[arg(long)]
        lamports_per_agent: Option<u64>,
        /// Maximum funding actions reconciled or executed in this bounded pass.
        #[arg(short, long, default_value_t = 1_000)]
        limit: usize,
        /// Permit local Surfpool submissions after policy checks.
        #[arg(long, requires = "acknowledge_policy")]
        execute: bool,
        /// Explicit acknowledgement required for Surfpool execution mode.
        #[arg(long, requires = "execute")]
        acknowledge_policy: bool,
    },
    /// Write a standalone, documented example configuration.
    Init {
        /// Destination configuration path.
        #[arg(short, long, default_value = "cooker.toml")]
        output: PathBuf,
        /// Atomically replace an existing destination.
        #[arg(long)]
        force: bool,
    },
    /// Parse and validate all offline and safety settings without network access.
    Validate {
        /// Configuration path.
        #[arg(short, long, default_value = "cooker.toml")]
        config: PathBuf,
    },
    /// Print a bounded preview of observable planned actions.
    Plan {
        /// Configuration path.
        #[arg(short, long, default_value = "cooker.toml")]
        config: PathBuf,
        /// Maximum rows printed.
        #[arg(short, long, default_value_t = 25)]
        limit: usize,
        /// Preview-agent count.
        #[arg(long, default_value_t = 8)]
        agents: usize,
        /// Preview duration in virtual days.
        #[arg(long, default_value_t = 7)]
        days: u16,
        /// Decimal u64 or 64-character hexadecimal seed override.
        #[arg(long)]
        seed: Option<String>,
    },
    /// Generate stable trace JSONL and a hash-bearing summary.
    Simulate {
        /// Configuration path.
        #[arg(short, long, default_value = "cooker.toml")]
        config: PathBuf,
        /// Directory receiving `trace.jsonl` and `summary.json`.
        #[arg(short, long, default_value = "artifacts/simulation")]
        output_dir: PathBuf,
        /// Agent-count override; the config default is canonical 1,000.
        #[arg(long)]
        agents: Option<usize>,
        /// Virtual-day override; the config default is canonical 30.
        #[arg(long)]
        days: Option<u16>,
        /// Decimal u64 or 64-character hexadecimal seed override.
        #[arg(long)]
        seed: Option<String>,
        /// RFC3339 simulation epoch override.
        #[arg(long)]
        start: Option<String>,
        /// Atomically replace existing artifacts.
        #[arg(long)]
        force: bool,
    },
    /// Run the configured five-seed comparative experiment and write three reports.
    Evaluate {
        /// Configuration path.
        #[arg(short, long, default_value = "cooker.toml")]
        config: PathBuf,
        /// Directory receiving JSON, CSV, and Markdown reports.
        #[arg(short, long, default_value = "artifacts/evaluation")]
        output_dir: PathBuf,
        /// Atomically replace existing artifacts.
        #[arg(long)]
        force: bool,
    },
    /// Run and independently replay a bounded, multi-seed virtual soak.
    Soak {
        /// Configuration path.
        #[arg(short, long, default_value = "cooker.toml")]
        config: PathBuf,
        /// Directory receiving per-seed traces and `manifest.json`.
        #[arg(short, long, default_value = "evidence/raw/virtual-soak")]
        output_dir: PathBuf,
        /// Comma-separated held-out decimal seeds; config evaluation seeds are the default.
        #[arg(long, value_delimiter = ',', num_args = 1..)]
        seeds: Option<Vec<u64>>,
        /// Agent-count override; canonical mode requires 1,000.
        #[arg(long)]
        agents: Option<usize>,
        /// Virtual-day override; canonical mode requires 30.
        #[arg(long)]
        days: Option<u16>,
        /// Permit reduced seed and workload bounds for local command tests.
        #[arg(long)]
        quick: bool,
        /// Verify an existing manifest and its streamed traces instead of simulating.
        #[arg(long, conflicts_with = "force")]
        verify_only: bool,
        /// Atomically replace existing soak artifacts.
        #[arg(long)]
        force: bool,
    },
    /// Verify Surfpool identity and health without loading a signer or changing state.
    Doctor {
        /// Configuration path.
        #[arg(short, long, default_value = "cooker.toml")]
        config: PathBuf,
    },
    /// Preview or execute one bounded, configured fleet pass on Surfpool.
    Run {
        /// Configuration path.
        #[arg(short, long, default_value = "cooker.toml")]
        config: PathBuf,
        /// Project root containing the ignored fleet manifest and signer files.
        #[arg(long, default_value = ".")]
        project_root: PathBuf,
        /// Runtime `SQLite` database.
        #[arg(long, default_value = ".surfpool/state/cooker.sqlite")]
        database: PathBuf,
        /// Maximum due actions claimed for this bounded pass.
        #[arg(short, long, default_value_t = 25)]
        limit: usize,
        /// Maximum durable planner decisions advanced before claiming work.
        #[arg(long, default_value_t = 10_000)]
        planning_limit: usize,
        /// Permit transaction submission after policy checks.
        #[arg(long, requires = "acknowledge_policy")]
        execute: bool,
        /// Explicit acknowledgement required for Surfpool execution mode.
        #[arg(long, requires = "execute")]
        acknowledge_policy: bool,
    },
    /// Inspect durable runtime state without network or signer access.
    Status {
        /// Configuration path.
        #[arg(short, long, default_value = "cooker.toml")]
        config: PathBuf,
        /// Project root used to resolve the database path.
        #[arg(long, default_value = ".")]
        project_root: PathBuf,
        /// Runtime `SQLite` database.
        #[arg(long, default_value = ".surfpool/state/cooker.sqlite")]
        database: PathBuf,
    },
    /// Preview or execute reconciliation without resubmitting transactions.
    Recover {
        /// Configuration path.
        #[arg(short, long, default_value = "cooker.toml")]
        config: PathBuf,
        /// Project root containing the ignored fleet manifest and signer files.
        #[arg(long, default_value = ".")]
        project_root: PathBuf,
        /// Runtime `SQLite` database.
        #[arg(long, default_value = ".surfpool/state/cooker.sqlite")]
        database: PathBuf,
        /// Maximum rows previewed or claimed in each recovery category.
        #[arg(short, long, default_value_t = 100)]
        limit: usize,
        /// Also re-audit confirmations at least this many seconds old.
        #[arg(long)]
        audit_age_seconds: Option<u64>,
        /// Apply recovery changes after Surfpool and signer preflight.
        #[arg(long, requires = "acknowledge_policy")]
        execute: bool,
        /// Explicit acknowledgement required for Surfpool recovery execution.
        #[arg(long, requires = "execute")]
        acknowledge_policy: bool,
    },
}

/// Execute one parsed command, writing normal output to the supplied stream.
pub fn execute(cli: Cli, output: &mut impl Write) -> Result<()> {
    commands::execute(cli.command, output)
}

/// Install newline-delimited JSON diagnostics on stderr for the CLI process.
///
/// `COOKER_LOG` controls the filter and defaults to `warn`. Command output remains isolated on
/// stdout so JSON reports stay machine-readable.
pub fn init_observability() -> Result<()> {
    let filter = match std::env::var("COOKER_LOG") {
        Ok(value) => EnvFilter::try_new(value)?,
        Err(std::env::VarError::NotPresent) => EnvFilter::new("warn"),
        Err(error) => return Err(error.into()),
    };
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(filter)
        .with_current_span(false)
        .with_span_list(false)
        .with_writer(std::io::stderr)
        .try_init()
        .map_err(|error| anyhow::anyhow!("failed to initialize structured logging: {error}"))?;
    Ok(())
}
