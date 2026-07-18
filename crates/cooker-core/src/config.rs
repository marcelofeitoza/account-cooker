//! Serializable configuration and safety validation.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use url::Url;

use crate::{domain::ActionKind, error::CookerError};

/// Complete application configuration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CookerConfig {
    /// Local Surfpool identity.
    pub network: NetworkConfig,
    /// Runtime concurrency and timing.
    pub runtime: RuntimeConfig,
    /// Spend and reserve limits.
    pub budgets: BudgetConfig,
    /// Explicitly enabled action families.
    pub enabled_actions: BTreeSet<ActionKind>,
    /// Master deterministic seed encoded as lowercase hex.
    pub seed_hex: String,
}

impl CookerConfig {
    /// Validate cross-field invariants that do not require network access.
    pub fn validate(&self) -> Result<(), CookerError> {
        self.network.validate_loopback()?;
        if self.runtime.max_concurrency == 0 {
            return Err(CookerError::InvalidConfig(
                "max_concurrency must be positive".to_owned(),
            ));
        }
        if self.runtime.lease_seconds == 0 {
            return Err(CookerError::InvalidConfig(
                "lease_seconds must be positive".to_owned(),
            ));
        }
        if self.budgets.daily_lamports > self.budgets.lifetime_lamports {
            return Err(CookerError::InvalidConfig(
                "daily budget cannot exceed lifetime budget".to_owned(),
            ));
        }
        if self.enabled_actions.is_empty() {
            return Err(CookerError::InvalidConfig(
                "at least one action must be enabled".to_owned(),
            ));
        }
        let bytes = hex::decode(&self.seed_hex)
            .map_err(|error| CookerError::InvalidConfig(format!("invalid seed hex: {error}")))?;
        if bytes.len() != 32 {
            return Err(CookerError::InvalidConfig(
                "seed must contain exactly 32 bytes".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Required local network identity.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NetworkConfig {
    /// Surfpool HTTP RPC endpoint.
    pub rpc_url: Url,
    /// Expected Surfnet identifier.
    pub surfnet_id: String,
}

impl NetworkConfig {
    /// Reject non-loopback endpoints before any signer may be loaded.
    pub fn validate_loopback(&self) -> Result<(), CookerError> {
        let is_loopback = self
            .rpc_url
            .host_str()
            .is_some_and(|host| matches!(host, "127.0.0.1" | "localhost" | "::1"));
        if self.rpc_url.scheme() != "http" || !is_loopback {
            return Err(CookerError::InvalidConfig(
                "RPC must be a loopback HTTP Surfpool endpoint".to_owned(),
            ));
        }
        if self.surfnet_id.trim().is_empty() {
            return Err(CookerError::InvalidConfig(
                "surfnet_id cannot be empty".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Runtime scheduler limits.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeConfig {
    /// Maximum simultaneous workers.
    pub max_concurrency: usize,
    /// Durable action lease duration.
    pub lease_seconds: u64,
    /// Stop claiming new work when true.
    #[serde(default)]
    pub kill_switch: bool,
}

/// Native-unit safety ceilings.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BudgetConfig {
    /// Maximum spend in one UTC day.
    pub daily_lamports: u64,
    /// Maximum spend over the run.
    pub lifetime_lamports: u64,
    /// Minimum balance retained by every signer.
    pub reserve_lamports: u64,
    /// Maximum transaction fee.
    pub max_fee_lamports: u64,
    /// Maximum durable lamports allocated to account creation by one action.
    pub max_account_creation_lamports: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_rpc_is_rejected() {
        let network = NetworkConfig {
            rpc_url: Url::parse("https://api.mainnet-beta.solana.com").unwrap(),
            surfnet_id: "noise-dev".to_owned(),
        };
        assert!(network.validate_loopback().is_err());
    }
}
