//! Standalone CLI configuration and deterministic override handling.

use std::{collections::BTreeSet, fs, path::Path};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use cooker_core::{
    ActionCatalog, ActionKind, CookerConfig, PersonaConfig, PersonaPreset, SimulationConfig,
    SplRoute, SwapRoute,
};
use cooker_eval::ExperimentConfig;
use serde::{Deserialize, Serialize};

/// Canonical example written by `cooker init`.
pub(crate) const EXAMPLE_CONFIG: &str = r#"# Account Cooker is offline-first. Chain commands remain restricted to the
# loopback Surfpool endpoint below when they are enabled.

[core]
enabled_actions = ["native_transfer", "spl_transfer", "jupiter_swap"]
seed_hex = "2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a"

[core.network]
rpc_url = "http://127.0.0.1:8899"
surfnet_id = "noise-dev"

[core.runtime]
max_concurrency = 64
lease_seconds = 45
kill_switch = false

[core.budgets]
daily_lamports = 500000000
lifetime_lamports = 15000000000
reserve_lamports = 50000000
max_fee_lamports = 10000
max_account_creation_lamports = 10000000

# Real runtime commands use this separate allowlist. The default demo fleet executes native
# transfers; SPL, Jupiter, and stake require explicit protocol-specific online configuration.
[online]
enabled_actions = ["native_transfer"]
confirmation_timeout_seconds = 30
confirmation_audit_age_seconds = 300
# `cooker fund` uses this as the direct amount or pooled fixed denomination.
# Direct funding is the default; pooled topology and round settings are explicit CLI options.
funding_lamports_per_agent = 600000000
kill_switch_file = ".surfpool/KILL_SWITCH"

[offline]
start = "2026-01-01T00:00:00Z"
simulation_agents = 1000
simulation_days = 30
max_decisions_per_agent_day = 500
max_slippage_bps = 100
personas = ["casual", "trader", "saver", "explorer"]
# These are stable simulation identifiers, not keys used for execution.
native_destinations = ["simulation-destination-00", "simulation-destination-01"]
allow_stake_enter = false

[[offline.spl_routes]]
mint = "simulation-mint-c"
destination_owner = "simulation-destination-00"

[[offline.swap_routes]]
input_mint = "So11111111111111111111111111111111111111112"
output_mint = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"

[[offline.swap_routes]]
input_mint = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"
output_mint = "So11111111111111111111111111111111111111112"

[evaluation]
controllers = 10
agents_per_controller = 10
days = 30
events_per_agent_per_day = 2
seeds = [11, 23, 37, 51, 71]
threshold = 0.55
"#;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct FileConfig {
    pub core: CookerConfig,
    pub offline: OfflineConfig,
    #[serde(default)]
    pub online: OnlineConfig,
    pub evaluation: EvaluationFileConfig,
}

impl FileConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path)
            .with_context(|| format!("failed to read configuration {}", path.display()))?;
        toml::from_str(&text)
            .with_context(|| format!("failed to parse configuration {}", path.display()))
    }

    pub fn validate(&self) -> Result<()> {
        self.core
            .validate()
            .map_err(anyhow::Error::msg)
            .context("core configuration is invalid")?;
        if self.core.budgets.daily_lamports == 0
            || self.core.budgets.lifetime_lamports == 0
            || self.core.budgets.max_fee_lamports == 0
        {
            bail!("core budgets and maximum fee must be positive");
        }
        self.offline.validate(&self.core.enabled_actions)?;
        self.online.validate(&self.core.enabled_actions)?;
        self.evaluation.validate_cli_bounds()?;
        self.evaluation
            .experiment()
            .validate()
            .map_err(anyhow::Error::msg)
            .context("evaluation configuration is invalid")
    }

    pub fn simulation(
        &self,
        agents: Option<usize>,
        days: Option<u16>,
        seed: Option<&str>,
        start: Option<&str>,
    ) -> Result<SimulationConfig> {
        let start = match start {
            Some(value) => parse_start(value)?,
            None => self.offline.start,
        };
        let agent_count = agents.unwrap_or(self.offline.simulation_agents);
        let config = SimulationConfig {
            start,
            days: days.unwrap_or(self.offline.simulation_days),
            agent_count,
            max_concurrency: self.core.runtime.max_concurrency.min(agent_count),
            max_decisions_per_agent_day: self.offline.max_decisions_per_agent_day,
            daily_budget: self.core.budgets.daily_lamports,
            max_fee_lamports: self.core.budgets.max_fee_lamports,
            max_account_creation_lamports: self.core.budgets.max_account_creation_lamports,
            max_slippage_bps: self.offline.max_slippage_bps,
            global_seed: parse_seed(seed.unwrap_or(&self.core.seed_hex))?,
            model_version: "behavior-v1".to_owned(),
        };
        config
            .validate()
            .map_err(anyhow::Error::msg)
            .context("simulation configuration is invalid")?;
        Ok(config)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct OnlineConfig {
    #[serde(default = "default_online_actions")]
    pub enabled_actions: BTreeSet<ActionKind>,
    #[serde(default = "default_confirmation_timeout_seconds")]
    pub confirmation_timeout_seconds: u64,
    #[serde(default = "default_confirmation_audit_age_seconds")]
    pub confirmation_audit_age_seconds: u64,
    #[serde(default = "default_funding_lamports_per_agent")]
    pub funding_lamports_per_agent: u64,
    #[serde(default = "default_kill_switch_file")]
    pub kill_switch_file: String,
    #[serde(default)]
    pub spl_routes: Vec<SplRoute>,
    #[serde(default)]
    pub swap_routes: Vec<SwapRoute>,
    #[serde(default)]
    pub stake_vote_account: Option<String>,
    #[serde(default)]
    pub jupiter: Option<JupiterFileConfig>,
}

impl Default for OnlineConfig {
    fn default() -> Self {
        Self {
            enabled_actions: default_online_actions(),
            confirmation_timeout_seconds: default_confirmation_timeout_seconds(),
            confirmation_audit_age_seconds: default_confirmation_audit_age_seconds(),
            funding_lamports_per_agent: default_funding_lamports_per_agent(),
            kill_switch_file: default_kill_switch_file(),
            spl_routes: Vec::new(),
            swap_routes: Vec::new(),
            stake_vote_account: None,
            jupiter: None,
        }
    }
}

impl OnlineConfig {
    fn validate(&self, modeled_actions: &BTreeSet<ActionKind>) -> Result<()> {
        if self.enabled_actions.is_empty()
            || self.enabled_actions.contains(&ActionKind::Idle)
            || !self.enabled_actions.is_subset(modeled_actions)
        {
            bail!(
                "online enabled actions must be a non-empty non-idle subset of core.enabled_actions"
            );
        }
        if !(1..=300).contains(&self.confirmation_timeout_seconds) {
            bail!("online confirmation timeout must be in 1..=300 seconds");
        }
        if self.confirmation_audit_age_seconds == 0 || self.funding_lamports_per_agent == 0 {
            bail!("online audit age and per-agent funding must be positive");
        }
        let kill_switch = Path::new(&self.kill_switch_file);
        if self.kill_switch_file.trim().is_empty()
            || kill_switch.is_absolute()
            || kill_switch.components().any(|component| {
                !matches!(
                    component,
                    std::path::Component::Normal(_) | std::path::Component::CurDir
                )
            })
        {
            bail!("online.kill_switch_file must be a non-empty project-relative path");
        }
        if self.enabled_actions.contains(&ActionKind::SplTransfer) && self.spl_routes.is_empty() {
            bail!("online SPL execution requires online.spl_routes");
        }
        if self
            .spl_routes
            .iter()
            .any(|route| route.mint.trim().is_empty() || route.destination_owner.trim().is_empty())
        {
            bail!("online SPL routes require non-empty mint and destination owner values");
        }
        if self.enabled_actions.contains(&ActionKind::JupiterSwap)
            && (self.swap_routes.is_empty() || self.jupiter.is_none())
        {
            bail!("online Jupiter execution requires online.swap_routes and online.jupiter");
        }
        if self.swap_routes.iter().any(|route| {
            route.input_mint.trim().is_empty()
                || route.output_mint.trim().is_empty()
                || route.input_mint == route.output_mint
        }) {
            bail!("online swap routes require distinct non-empty mints");
        }
        if self.enabled_actions.contains(&ActionKind::StakeLifecycle)
            && self
                .stake_vote_account
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
        {
            bail!("online stake execution requires online.stake_vote_account");
        }
        if let Some(jupiter) = &self.jupiter {
            jupiter.validate()?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct JupiterFileConfig {
    pub route_label: String,
    pub allowed_amm_accounts: Vec<String>,
    pub allowed_programs: Vec<String>,
    #[serde(default)]
    pub allowed_accounts: Vec<String>,
    #[serde(default)]
    pub allowed_writable_accounts: Vec<String>,
    pub allowed_lookup_tables: Vec<String>,
    pub max_price_impact_bps: u16,
    #[serde(default)]
    pub api_key_env: Option<String>,
}

impl JupiterFileConfig {
    fn validate(&self) -> Result<()> {
        if self.route_label.trim().is_empty()
            || self.allowed_amm_accounts.is_empty()
            || self.allowed_programs.is_empty()
            || self.allowed_lookup_tables.is_empty()
            || self.max_price_impact_bps > 10_000
        {
            bail!(
                "online Jupiter policy requires a route label, reviewed AMM/program/lookup allowlists, and a valid price-impact limit"
            );
        }
        if self
            .api_key_env
            .as_deref()
            .is_some_and(|name| !valid_environment_name(name))
        {
            bail!("online Jupiter api_key_env is not a valid environment variable name");
        }
        for value in self
            .allowed_amm_accounts
            .iter()
            .chain(&self.allowed_programs)
            .chain(&self.allowed_accounts)
            .chain(&self.allowed_writable_accounts)
            .chain(&self.allowed_lookup_tables)
        {
            if value.trim().is_empty() {
                bail!("online Jupiter allowlists cannot contain empty values");
            }
        }
        Ok(())
    }
}

fn default_online_actions() -> BTreeSet<ActionKind> {
    BTreeSet::from([ActionKind::NativeTransfer])
}

const fn default_confirmation_timeout_seconds() -> u64 {
    30
}

const fn default_confirmation_audit_age_seconds() -> u64 {
    300
}

const fn default_funding_lamports_per_agent() -> u64 {
    600_000_000
}

fn default_kill_switch_file() -> String {
    ".surfpool/KILL_SWITCH".to_owned()
}

fn valid_environment_name(value: &str) -> bool {
    let mut characters = value.chars();
    characters
        .next()
        .is_some_and(|character| character == '_' || character.is_ascii_alphabetic())
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct OfflineConfig {
    pub start: DateTime<Utc>,
    pub simulation_agents: usize,
    pub simulation_days: u16,
    pub max_decisions_per_agent_day: u32,
    pub max_slippage_bps: u16,
    pub personas: Vec<PersonaPreset>,
    #[serde(default)]
    pub native_destinations: Vec<String>,
    #[serde(default)]
    pub spl_routes: Vec<SplRoute>,
    #[serde(default)]
    pub swap_routes: Vec<SwapRoute>,
    #[serde(default)]
    pub allow_stake_enter: bool,
}

impl OfflineConfig {
    fn validate(&self, enabled: &BTreeSet<ActionKind>) -> Result<()> {
        if self.simulation_agents == 0
            || self.simulation_agents > 10_000
            || self.simulation_days == 0
            || self.simulation_days > 365
            || self.max_decisions_per_agent_day == 0
        {
            bail!("offline simulation requires agents in 1..=10000 and days in 1..=365");
        }
        let agent_days = self
            .simulation_agents
            .checked_mul(usize::from(self.simulation_days))
            .context("offline simulation size overflow")?;
        if agent_days > 100_000 {
            bail!("offline simulation is limited to 100000 agent-days");
        }
        if self.personas.is_empty() {
            bail!("offline.personas must contain at least one preset");
        }
        for persona in self.persona_configs() {
            persona
                .validate()
                .map_err(anyhow::Error::msg)
                .context("offline persona is invalid")?;
        }
        if enabled.contains(&ActionKind::NativeTransfer) && self.native_destinations.is_empty() {
            bail!("enabled native transfers require offline.native_destinations");
        }
        if enabled.contains(&ActionKind::SplTransfer) && self.spl_routes.is_empty() {
            bail!("enabled SPL transfers require offline.spl_routes");
        }
        if enabled.contains(&ActionKind::JupiterSwap) && self.swap_routes.is_empty() {
            bail!("enabled Jupiter swaps require offline.swap_routes");
        }
        if enabled.contains(&ActionKind::StakeLifecycle) && !self.allow_stake_enter {
            bail!("enabled stake lifecycle requires offline.allow_stake_enter");
        }
        self.catalog(enabled)
            .validate()
            .map_err(anyhow::Error::msg)
            .context("offline action catalog is invalid")
    }

    pub fn persona_configs(&self) -> Vec<PersonaConfig> {
        self.personas
            .iter()
            .copied()
            .map(PersonaConfig::preset)
            .collect()
    }

    pub fn catalog(&self, enabled: &BTreeSet<ActionKind>) -> ActionCatalog {
        ActionCatalog {
            native_destinations: if enabled.contains(&ActionKind::NativeTransfer) {
                self.native_destinations.clone()
            } else {
                Vec::new()
            },
            spl_routes: if enabled.contains(&ActionKind::SplTransfer) {
                self.spl_routes.clone()
            } else {
                Vec::new()
            },
            swap_routes: if enabled.contains(&ActionKind::JupiterSwap) {
                self.swap_routes.clone()
            } else {
                Vec::new()
            },
            allow_stake_enter: self.allow_stake_enter
                && enabled.contains(&ActionKind::StakeLifecycle),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct EvaluationFileConfig {
    pub controllers: usize,
    pub agents_per_controller: usize,
    pub days: u32,
    pub events_per_agent_per_day: u32,
    pub seeds: Vec<u64>,
    pub threshold: f64,
}

impl EvaluationFileConfig {
    pub fn experiment(&self) -> ExperimentConfig {
        ExperimentConfig {
            controllers: self.controllers,
            agents_per_controller: self.agents_per_controller,
            days: self.days,
            events_per_agent_per_day: self.events_per_agent_per_day,
            seeds: self.seeds.clone(),
            threshold: self.threshold,
            ..ExperimentConfig::default()
        }
    }

    fn validate_cli_bounds(&self) -> Result<()> {
        if self.seeds.len() != 5 {
            bail!("evaluation requires exactly five held-out seeds");
        }
        let agents = self
            .controllers
            .checked_mul(self.agents_per_controller)
            .context("evaluation agent count overflow")?;
        if agents > 1_000 {
            bail!("evaluation is limited to 1000 total agents");
        }
        let events = agents
            .checked_mul(usize::try_from(self.days).context("evaluation day count overflow")?)
            .and_then(|value| {
                value.checked_mul(usize::try_from(self.events_per_agent_per_day).ok()?)
            })
            .context("evaluation event count overflow")?;
        if events > 1_000_000 {
            bail!("evaluation is limited to 1000000 events per planner/seed");
        }
        Ok(())
    }
}

pub(crate) fn parse_seed(value: &str) -> Result<[u8; 32]> {
    if value.len() == 64 {
        let decoded = hex::decode(value).context("seed is not valid hexadecimal")?;
        return decoded
            .try_into()
            .map_err(|_| anyhow::anyhow!("hexadecimal seed must contain exactly 32 bytes"));
    }
    let numeric = value
        .parse::<u64>()
        .context("seed must be a decimal u64 or 64 hexadecimal characters")?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"account-cooker-cli-seed-v1");
    hasher.update(&numeric.to_le_bytes());
    Ok(*hasher.finalize().as_bytes())
}

fn parse_start(value: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .with_context(|| format!("invalid RFC3339 simulation start {value:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_is_standalone_and_valid() -> Result<()> {
        let config: FileConfig = toml::from_str(EXAMPLE_CONFIG)?;
        config.validate()
    }

    #[test]
    fn decimal_and_hex_seed_forms_are_stable() -> Result<()> {
        assert_eq!(parse_seed("42")?, parse_seed("42")?);
        assert_eq!(parse_seed(&"2a".repeat(32))?, [42; 32]);
        assert!(parse_seed("not-a-seed").is_err());
        Ok(())
    }
}
