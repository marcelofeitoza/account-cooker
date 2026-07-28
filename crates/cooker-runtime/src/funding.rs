//! Runtime action materialization for deterministic funding schedules.

use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

use chrono::{DateTime, TimeDelta, Utc};
use cooker_core::{
    ActionId, ActionPayload, AgentId, CookerError, FundingPlan, FundingScheme, PlannedAction, RunId,
};

/// Chain destination bound to one account in a funding schedule.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FundingRecipient {
    /// Stable account identifier consumed by the schedule builder.
    pub account: AgentId,
    /// Native transfer destination for this account.
    pub destination: String,
}

/// Exact spend ceiling assigned to one disburser runtime.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DisburserBudget {
    /// Scheduled native transfers signed by this disburser.
    pub transfers: usize,
    /// Native principal across those transfers.
    pub principal_lamports: u64,
    /// Sum of the per-transaction fee ceilings.
    pub fee_ceiling_lamports: u64,
}

impl DisburserBudget {
    /// Principal plus every transaction's fee ceiling.
    ///
    /// # Errors
    ///
    /// Returns an error when the exact ceiling does not fit `u64`.
    pub fn total_lamports(self) -> Result<u64, CookerError> {
        self.principal_lamports
            .checked_add(self.fee_ceiling_lamports)
            .ok_or_else(|| CookerError::InvalidConfig("disburser budget overflowed".to_owned()))
    }
}

/// Durable native-transfer actions produced from one core funding schedule.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FundingExecutionPlan {
    scheme: FundingScheme,
    actions: Vec<PlannedAction>,
    disburser_budgets: BTreeMap<AgentId, DisburserBudget>,
}

impl FundingExecutionPlan {
    /// Materialize the core payer assignment as durable runtime actions.
    ///
    /// The core schedule supplies round, payer, account, and denomination. This bridge binds
    /// payer indexes to durable runtime agents, account identifiers to chain destinations, and
    /// integer rounds to wall-clock times. A nanosecond offset preserves the core order within
    /// each round when the store claims due actions.
    ///
    /// # Errors
    ///
    /// Returns an error for incomplete or duplicate bindings, an invalid round interval, missing
    /// payer indexes, arithmetic overflow, or an empty model version.
    #[allow(
        clippy::too_many_arguments,
        clippy::too_many_lines,
        reason = "the bridge keeps binding, ordering, identity, and budget checks in one pass"
    )]
    pub fn new(
        schedule: &FundingPlan,
        recipients: &[FundingRecipient],
        disburser_agents: &[AgentId],
        run_id: RunId,
        model_version: &str,
        epoch: DateTime<Utc>,
        round_interval: Duration,
        max_fee_lamports: u64,
    ) -> Result<Self, CookerError> {
        if model_version.trim().is_empty() {
            return Err(CookerError::InvalidConfig(
                "funding model version cannot be empty".to_owned(),
            ));
        }
        if disburser_agents.is_empty()
            || disburser_agents
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
                .len()
                != disburser_agents.len()
        {
            return Err(CookerError::InvalidConfig(
                "funding disburser agents must be non-empty and unique".to_owned(),
            ));
        }
        let ordered = schedule.ordered_top_ups();
        let expected_disbursers = match schedule.scheme() {
            FundingScheme::PooledMixedRounds | FundingScheme::PooledPerOperatorRounds => {
                schedule.config().disbursers
            }
            FundingScheme::DedicatedPerOperator => ordered
                .iter()
                .map(|top_up| top_up.funder)
                .max()
                .map_or(0, |funder| funder.saturating_add(1)),
        };
        if disburser_agents.len() != expected_disbursers {
            return Err(CookerError::InvalidConfig(
                "funding disburser bindings must exactly match the schedule".to_owned(),
            ));
        }
        let recipient_map = recipient_bindings(recipients)?;
        let scheduled_accounts: BTreeSet<_> = ordered.iter().map(|top_up| top_up.account).collect();
        let bound_accounts: BTreeSet<_> = recipient_map.keys().copied().collect();
        if scheduled_accounts != bound_accounts {
            return Err(CookerError::InvalidConfig(
                "funding recipients must exactly match the scheduled accounts".to_owned(),
            ));
        }
        validate_round_interval(&ordered, schedule.config().rounds, round_interval)?;
        let round_delta = TimeDelta::from_std(round_interval).map_err(|_| {
            CookerError::InvalidConfig("funding round interval is too large".to_owned())
        })?;

        let mut sequences: BTreeMap<AgentId, u64> = disburser_agents
            .iter()
            .copied()
            .map(|agent| (agent, 0))
            .collect();
        let mut budgets: BTreeMap<AgentId, DisburserBudget> = disburser_agents
            .iter()
            .copied()
            .map(|agent| (agent, DisburserBudget::default()))
            .collect();
        let mut actions = Vec::with_capacity(ordered.len());
        let mut active_round = None;
        let mut round_position = 0_i64;

        for top_up in ordered {
            if active_round != Some(top_up.round) {
                active_round = Some(top_up.round);
                round_position = 0;
            }
            let agent_id = disburser_agents
                .get(top_up.funder)
                .copied()
                .ok_or_else(|| {
                    CookerError::InvalidConfig(format!(
                        "funding schedule references missing disburser {}",
                        top_up.funder
                    ))
                })?;
            let destination = recipient_map.get(&top_up.account).ok_or_else(|| {
                CookerError::InvalidConfig(format!(
                    "funding schedule references missing account {}",
                    top_up.account
                ))
            })?;
            let round_multiplier = i32::try_from(top_up.round).map_err(|_| {
                CookerError::InvalidConfig("funding round does not fit time multiplier".to_owned())
            })?;
            let scheduled_at = epoch
                .checked_add_signed(round_delta.checked_mul(round_multiplier).ok_or_else(|| {
                    CookerError::InvalidConfig("funding round time overflowed".to_owned())
                })?)
                .and_then(|time| time.checked_add_signed(TimeDelta::nanoseconds(round_position)))
                .ok_or_else(|| {
                    CookerError::InvalidConfig("funding action time overflowed".to_owned())
                })?;
            round_position = round_position.checked_add(1).ok_or_else(|| {
                CookerError::InvalidConfig("funding round position overflowed".to_owned())
            })?;
            let sequence = sequences.get_mut(&agent_id).ok_or_else(|| {
                CookerError::InvalidConfig("funding disburser sequence is missing".to_owned())
            })?;
            let action = PlannedAction {
                id: ActionId::derive(run_id, agent_id, *sequence, model_version),
                run_id,
                agent_id,
                sequence: *sequence,
                model_version: model_version.to_owned(),
                scheduled_at,
                payload: ActionPayload::NativeTransfer {
                    destination: destination.clone(),
                    lamports: top_up.lamports,
                },
                max_fee_lamports,
                max_account_creation_lamports: 0,
                created_at: epoch,
            };
            *sequence = sequence.checked_add(1).ok_or_else(|| {
                CookerError::InvalidConfig("funding action sequence overflowed".to_owned())
            })?;
            let budget = budgets.get_mut(&agent_id).ok_or_else(|| {
                CookerError::InvalidConfig("funding disburser budget is missing".to_owned())
            })?;
            budget.transfers = budget.transfers.checked_add(1).ok_or_else(|| {
                CookerError::InvalidConfig("funding transfer count overflowed".to_owned())
            })?;
            budget.principal_lamports = budget
                .principal_lamports
                .checked_add(top_up.lamports)
                .ok_or_else(|| {
                    CookerError::InvalidConfig("funding principal overflowed".to_owned())
                })?;
            budget.fee_ceiling_lamports = budget
                .fee_ceiling_lamports
                .checked_add(max_fee_lamports)
                .ok_or_else(|| {
                    CookerError::InvalidConfig("funding fee ceiling overflowed".to_owned())
                })?;
            actions.push(action);
        }

        Ok(Self {
            scheme: schedule.scheme(),
            actions,
            disburser_budgets: budgets,
        })
    }

    /// Payer assignment strategy inherited from the core schedule.
    #[must_use]
    pub const fn scheme(&self) -> FundingScheme {
        self.scheme
    }

    /// Actions ordered by round, payer, and account.
    #[must_use]
    pub fn actions(&self) -> &[PlannedAction] {
        &self.actions
    }

    /// Exact principal and fee ceiling for every durable disburser agent.
    #[must_use]
    pub const fn disburser_budgets(&self) -> &BTreeMap<AgentId, DisburserBudget> {
        &self.disburser_budgets
    }
}

fn recipient_bindings(
    recipients: &[FundingRecipient],
) -> Result<BTreeMap<AgentId, String>, CookerError> {
    if recipients.is_empty() {
        return Err(CookerError::InvalidConfig(
            "funding recipients cannot be empty".to_owned(),
        ));
    }
    let mut accounts = BTreeMap::new();
    let mut destinations = BTreeSet::new();
    for recipient in recipients {
        if recipient.destination.trim().is_empty() {
            return Err(CookerError::InvalidConfig(
                "funding destination cannot be empty".to_owned(),
            ));
        }
        if accounts
            .insert(recipient.account, recipient.destination.clone())
            .is_some()
        {
            return Err(CookerError::InvalidConfig(
                "funding account bindings must be unique".to_owned(),
            ));
        }
        if !destinations.insert(recipient.destination.as_str()) {
            return Err(CookerError::InvalidConfig(
                "funding destination bindings must be unique".to_owned(),
            ));
        }
    }
    Ok(accounts)
}

fn validate_round_interval(
    top_ups: &[cooker_core::FundingTopUp],
    rounds: u32,
    round_interval: Duration,
) -> Result<(), CookerError> {
    if rounds > 1 && round_interval.is_zero() {
        return Err(CookerError::InvalidConfig(
            "multi-round funding requires a positive round interval".to_owned(),
        ));
    }
    let largest_round = top_ups
        .iter()
        .fold(BTreeMap::<u32, usize>::new(), |mut counts, top_up| {
            *counts.entry(top_up.round).or_insert(0) += 1;
            counts
        })
        .into_values()
        .max()
        .unwrap_or(0);
    if rounds > 1
        && u128::try_from(largest_round).map_err(|_| {
            CookerError::InvalidConfig("funding round size does not fit u128".to_owned())
        })? > round_interval.as_nanos()
    {
        return Err(CookerError::InvalidConfig(
            "funding round interval cannot preserve transfer ordering".to_owned(),
        ));
    }
    Ok(())
}
