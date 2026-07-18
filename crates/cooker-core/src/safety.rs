//! Pure deny-by-default execution policy.

use std::collections::{BTreeMap, BTreeSet};

use chrono::Days;
use serde::{Deserialize, Serialize};

use crate::{
    config::BudgetConfig,
    contracts::Policy,
    domain::{ActionKind, ActionPayload, PlannedAction, PolicyDecision, StakeOperation},
    error::CookerError,
};

/// Complete safety policy configuration.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PolicyConfig {
    /// Native balance and fee ceilings.
    pub budgets: BudgetConfig,
    /// Enabled action families. Idle is always accepted at zero fee.
    pub allowed_actions: BTreeSet<ActionKind>,
    /// Maximum raw input by action family.
    pub max_action_amounts: BTreeMap<ActionKind, u64>,
    /// Exact operator-approved destination addresses.
    pub allowed_destinations: BTreeSet<String>,
    /// Exact operator-approved token mints.
    pub allowed_mints: BTreeSet<String>,
    /// Mints whose Jupiter input consumes the signer's native balance.
    pub native_input_mints: BTreeSet<String>,
    /// Maximum permitted Jupiter slippage.
    pub max_slippage_bps: u16,
}

impl PolicyConfig {
    /// Validate policy completeness before any action is evaluated.
    pub fn validate(&self) -> Result<(), CookerError> {
        if self.budgets.daily_lamports == 0
            || self.budgets.lifetime_lamports == 0
            || self.budgets.max_fee_lamports == 0
        {
            return Err(CookerError::InvalidConfig(
                "policy budgets and maximum fee must be positive".to_owned(),
            ));
        }
        if self.budgets.daily_lamports > self.budgets.lifetime_lamports {
            return Err(CookerError::InvalidConfig(
                "daily budget cannot exceed lifetime budget".to_owned(),
            ));
        }
        if self.max_slippage_bps > 10_000 {
            return Err(CookerError::InvalidConfig(
                "policy slippage cannot exceed 10000 basis points".to_owned(),
            ));
        }
        if !self
            .allowed_actions
            .iter()
            .any(|kind| *kind != ActionKind::Idle)
        {
            return Err(CookerError::InvalidConfig(
                "policy requires at least one non-idle action".to_owned(),
            ));
        }
        if self
            .allowed_destinations
            .iter()
            .any(|destination| destination.trim().is_empty())
            || self.allowed_mints.iter().any(|mint| mint.trim().is_empty())
            || !self.native_input_mints.is_subset(&self.allowed_mints)
        {
            return Err(CookerError::InvalidConfig(
                "policy allowlists contain an empty or inconsistent entry".to_owned(),
            ));
        }
        for kind in &self.allowed_actions {
            if *kind != ActionKind::Idle
                && self
                    .max_action_amounts
                    .get(kind)
                    .is_none_or(|maximum| *maximum == 0)
            {
                return Err(CookerError::InvalidConfig(format!(
                    "allowed action {kind:?} requires a positive amount cap"
                )));
            }
        }
        if self.allowed_actions.contains(&ActionKind::NativeTransfer)
            && self.allowed_destinations.is_empty()
        {
            return Err(CookerError::InvalidConfig(
                "native transfers require an explicit destination allowlist".to_owned(),
            ));
        }
        if self.allowed_actions.contains(&ActionKind::SplTransfer)
            && (self.allowed_destinations.is_empty() || self.allowed_mints.is_empty())
        {
            return Err(CookerError::InvalidConfig(
                "SPL transfers require destination and mint allowlists".to_owned(),
            ));
        }
        if self.allowed_actions.contains(&ActionKind::JupiterSwap) && self.allowed_mints.len() < 2 {
            return Err(CookerError::InvalidConfig(
                "Jupiter swaps require at least two allowed mints".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Validated safety policy implementing the shared pure contract.
#[derive(Clone, Debug)]
pub struct SafetyPolicy {
    config: PolicyConfig,
}

impl SafetyPolicy {
    /// Construct a deny-by-default policy.
    pub fn new(config: PolicyConfig) -> Result<Self, CookerError> {
        config.validate()?;
        Ok(Self { config })
    }

    /// Return the validated configuration.
    #[must_use]
    pub const fn config(&self) -> &PolicyConfig {
        &self.config
    }

    fn reject(reason: &str) -> PolicyDecision {
        PolicyDecision::Reject {
            reason: reason.to_owned(),
        }
    }

    fn validate_payload(&self, payload: &ActionPayload) -> Option<PolicyDecision> {
        match payload {
            ActionPayload::NativeTransfer { destination, .. } => {
                if !self.config.allowed_destinations.contains(destination) {
                    return Some(Self::reject("destination_not_allowed"));
                }
            }
            ActionPayload::SplTransfer {
                mint,
                destination_owner,
                ..
            } => {
                if !self.config.allowed_mints.contains(mint) {
                    return Some(Self::reject("mint_not_allowed"));
                }
                if !self.config.allowed_destinations.contains(destination_owner) {
                    return Some(Self::reject("destination_not_allowed"));
                }
            }
            ActionPayload::JupiterSwap {
                input_mint,
                output_mint,
                max_slippage_bps,
                ..
            } => {
                if input_mint == output_mint
                    || !self.config.allowed_mints.contains(input_mint)
                    || !self.config.allowed_mints.contains(output_mint)
                {
                    return Some(Self::reject("swap_pair_not_allowed"));
                }
                if *max_slippage_bps > self.config.max_slippage_bps {
                    return Some(Self::reject("slippage_exceeds_cap"));
                }
            }
            ActionPayload::StakeLifecycle {
                operation,
                lamports,
                stake_account,
            } => match operation {
                StakeOperation::Enter if *lamports > 0 && stake_account.is_none() => {}
                StakeOperation::Deactivate | StakeOperation::Withdraw
                    if *lamports == 0
                        && stake_account.as_deref().is_some_and(|v| !v.is_empty()) => {}
                _ => return Some(Self::reject("invalid_stake_lifecycle_payload")),
            },
            ActionPayload::Idle => {}
        }
        None
    }

    fn native_spend(&self, payload: &ActionPayload) -> u64 {
        match payload {
            ActionPayload::NativeTransfer { lamports, .. }
            | ActionPayload::StakeLifecycle {
                operation: StakeOperation::Enter,
                lamports,
                ..
            } => *lamports,
            ActionPayload::JupiterSwap {
                input_mint, amount, ..
            } if self.config.native_input_mints.contains(input_mint) => *amount,
            _ => 0,
        }
    }
}

impl Policy for SafetyPolicy {
    fn evaluate(
        &self,
        action: &PlannedAction,
        signer_balance: u64,
        spent_today: u64,
        spent_lifetime: u64,
    ) -> Result<PolicyDecision, CookerError> {
        let kind = action.payload.kind();
        if kind == ActionKind::Idle {
            return Ok(
                if action.max_fee_lamports == 0 && action.max_account_creation_lamports == 0 {
                    PolicyDecision::Allow
                } else {
                    Self::reject("idle_action_has_fee")
                },
            );
        }
        if !self.config.allowed_actions.contains(&kind) {
            return Ok(Self::reject("action_not_allowed"));
        }
        if let Some(rejection) = self.validate_payload(&action.payload) {
            return Ok(rejection);
        }
        if action.max_fee_lamports > self.config.budgets.max_fee_lamports {
            return Ok(Self::reject("fee_exceeds_cap"));
        }
        if action.max_account_creation_lamports > self.config.budgets.max_account_creation_lamports
        {
            return Ok(Self::reject("account_creation_exceeds_cap"));
        }
        let amount = action.payload.input_amount();
        if self
            .config
            .max_action_amounts
            .get(&kind)
            .is_none_or(|maximum| amount > *maximum)
        {
            return Ok(Self::reject("amount_exceeds_action_cap"));
        }

        let native_required = self
            .native_spend(&action.payload)
            .checked_add(action.max_fee_lamports)
            .and_then(|required| required.checked_add(action.max_account_creation_lamports))
            .ok_or_else(|| CookerError::Policy("native requirement overflow".to_owned()))?;
        let available = signer_balance.saturating_sub(self.config.budgets.reserve_lamports);
        if native_required > available {
            return Ok(Self::reject("reserve_or_balance_would_be_violated"));
        }
        let projected_lifetime = spent_lifetime
            .checked_add(native_required)
            .ok_or_else(|| CookerError::Policy("lifetime spend overflow".to_owned()))?;
        if projected_lifetime > self.config.budgets.lifetime_lamports {
            return Ok(Self::reject("lifetime_budget_exceeded"));
        }
        let projected_daily = spent_today
            .checked_add(native_required)
            .ok_or_else(|| CookerError::Policy("daily spend overflow".to_owned()))?;
        if projected_daily > self.config.budgets.daily_lamports {
            let tomorrow = action
                .scheduled_at
                .date_naive()
                .checked_add_days(Days::new(1))
                .and_then(|date| date.and_hms_opt(0, 0, 0))
                .ok_or_else(|| CookerError::Policy("daily reset time overflow".to_owned()))?
                .and_utc();
            return Ok(PolicyDecision::Delay {
                until: tomorrow,
                reason: "daily_budget_exceeded".to_owned(),
            });
        }
        Ok(PolicyDecision::Allow)
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use proptest::prelude::*;
    use uuid::Uuid;

    use super::*;
    use crate::domain::{ActionId, AgentId, RunId};

    fn policy() -> SafetyPolicy {
        SafetyPolicy::new(PolicyConfig {
            budgets: BudgetConfig {
                daily_lamports: 100_000_000,
                lifetime_lamports: 500_000_000,
                reserve_lamports: 10_000_000,
                max_fee_lamports: 10_000,
                max_account_creation_lamports: 10_000_000,
            },
            allowed_actions: [ActionKind::NativeTransfer, ActionKind::JupiterSwap]
                .into_iter()
                .collect(),
            max_action_amounts: [
                (ActionKind::NativeTransfer, 50_000_000),
                (ActionKind::JupiterSwap, 50_000_000),
            ]
            .into_iter()
            .collect(),
            allowed_destinations: ["destination-a".to_owned()].into_iter().collect(),
            allowed_mints: ["mint-a".to_owned(), "mint-b".to_owned()]
                .into_iter()
                .collect(),
            native_input_mints: ["mint-a".to_owned()].into_iter().collect(),
            max_slippage_bps: 100,
        })
        .unwrap()
    }

    fn action(payload: ActionPayload) -> PlannedAction {
        let run = RunId(Uuid::nil());
        let agent = AgentId(Uuid::from_u128(1));
        PlannedAction {
            id: ActionId::derive(run, agent, 0, "v1"),
            run_id: run,
            agent_id: agent,
            sequence: 0,
            model_version: "v1".to_owned(),
            scheduled_at: Utc.with_ymd_and_hms(2026, 7, 17, 12, 0, 0).unwrap(),
            payload,
            max_fee_lamports: 5_000,
            max_account_creation_lamports: 0,
            created_at: Utc.with_ymd_and_hms(2026, 7, 17, 11, 0, 0).unwrap(),
        }
    }

    #[test]
    fn reserve_is_never_consumed() {
        let decision = policy()
            .evaluate(
                &action(ActionPayload::NativeTransfer {
                    destination: "destination-a".to_owned(),
                    lamports: 20_000_000,
                }),
                25_000_000,
                0,
                0,
            )
            .unwrap();
        assert!(matches!(decision, PolicyDecision::Reject { .. }));
    }

    #[test]
    fn daily_limit_delays_without_weakening_lifetime_limit() {
        let decision = policy()
            .evaluate(
                &action(ActionPayload::NativeTransfer {
                    destination: "destination-a".to_owned(),
                    lamports: 20_000_000,
                }),
                1_000_000_000,
                90_000_000,
                100_000_000,
            )
            .unwrap();
        assert!(matches!(decision, PolicyDecision::Delay { .. }));
    }

    #[test]
    fn destination_and_mint_are_deny_by_default() {
        let denied_destination = policy()
            .evaluate(
                &action(ActionPayload::NativeTransfer {
                    destination: "unknown".to_owned(),
                    lamports: 1_000_000,
                }),
                1_000_000_000,
                0,
                0,
            )
            .unwrap();
        let denied_mint = policy()
            .evaluate(
                &action(ActionPayload::JupiterSwap {
                    input_mint: "unknown".to_owned(),
                    output_mint: "mint-b".to_owned(),
                    amount: 1_000_000,
                    max_slippage_bps: 50,
                }),
                1_000_000_000,
                0,
                0,
            )
            .unwrap();
        assert!(matches!(denied_destination, PolicyDecision::Reject { .. }));
        assert!(matches!(denied_mint, PolicyDecision::Reject { .. }));
    }

    #[test]
    fn action_amount_and_fee_caps_hold_across_a_range() {
        let policy = policy();
        for amount in [1, 50_000_000, 50_000_001, u64::MAX] {
            let decision = policy
                .evaluate(
                    &action(ActionPayload::NativeTransfer {
                        destination: "destination-a".to_owned(),
                        lamports: amount,
                    }),
                    u64::MAX,
                    0,
                    0,
                )
                .unwrap();
            assert_eq!(
                matches!(decision, PolicyDecision::Allow),
                amount <= 50_000_000
            );
        }
    }

    proptest! {
        #[test]
        fn allow_decisions_conserve_every_native_budget(
            amount in 0_u64..=60_000_000,
            fee in 0_u64..=20_000,
            account_creation in 0_u64..=20_000_000,
            signer_balance in 0_u64..=700_000_000,
            spent_today in 0_u64..=120_000_000,
            spent_lifetime in 0_u64..=550_000_000,
        ) {
            let policy = policy();
            let mut candidate = action(ActionPayload::NativeTransfer {
                destination: "destination-a".to_owned(),
                lamports: amount,
            });
            candidate.max_fee_lamports = fee;
            candidate.max_account_creation_lamports = account_creation;
            let decision = policy
                .evaluate(&candidate, signer_balance, spent_today, spent_lifetime)
                .map_err(|error| TestCaseError::fail(error.to_string()))?;

            if matches!(decision, PolicyDecision::Allow) {
                let required = amount + fee + account_creation;
                prop_assert!(amount <= 50_000_000);
                prop_assert!(fee <= 10_000);
                prop_assert!(account_creation <= 10_000_000);
                prop_assert!(required <= signer_balance.saturating_sub(10_000_000));
                prop_assert!(spent_today + required <= 100_000_000);
                prop_assert!(spent_lifetime + required <= 500_000_000);
            }
        }
    }
}
