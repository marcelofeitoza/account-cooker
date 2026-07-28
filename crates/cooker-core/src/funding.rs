//! Deterministic funding schedules for persona provisioning.
//!
//! The default engine funding path pays every persona from one operator wallet. That leaves
//! a funding edge which identifies the operator directly, and the evaluator measures it as a
//! perfectly separating signal. This module builds the schedules that answer it: uniform
//! denominations, batched rounds, and a shared disburser set whose per-round assignment
//! never reads an operator label.
//!
//! Three schemes share one participation schedule so that only the payer assignment differs
//! between them. [`FundingScheme::PooledMixedRounds`] is the mitigation,
//! [`FundingScheme::PooledPerOperatorRounds`] is the control that keeps the shared pool but
//! batches each operator separately, and [`FundingScheme::DedicatedPerOperator`] is the
//! baseline star topology.
//!
//! The evaluator measures these schedules, and the runtime funding coordinator consumes the
//! same [`FundingPlan`] to build durable native transfers. This module remains policy-only:
//! payer indexes become local signer routes in the runtime, while pool deposits, custody, and
//! transfer execution stay outside this crate.

use std::collections::{BTreeMap, BTreeSet};

use rand::{Rng, seq::SliceRandom};
use rand_chacha::{ChaCha12Rng, rand_core::SeedableRng};
use serde::{Deserialize, Serialize};

use crate::{domain::AgentId, error::CookerError};

/// Domain separator that keeps schedule randomness independent of planner randomness.
const SCHEDULE_VERSION: &str = "pooled-funding-v1";
/// Largest supported shared disburser set.
const MAX_DISBURSERS: usize = 4_096;
/// Largest supported funding horizon.
const MAX_ROUNDS: u32 = 100_000;

/// Uniform-denomination pooled funding parameters.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PooledFundingConfig {
    /// Shared wallets the pool disburses from.
    pub disbursers: usize,
    /// Batching rounds in the funding horizon.
    pub rounds: u32,
    /// Top-ups every account receives, including the provisioning round.
    pub top_ups_per_account: usize,
    /// Fixed lamport amount transferred by every top-up.
    pub denomination_lamports: u64,
}

impl PooledFundingConfig {
    /// Reject parameters that cannot produce a schedule.
    pub fn validate(&self) -> Result<(), CookerError> {
        if self.disbursers == 0 || self.disbursers > MAX_DISBURSERS {
            return Err(CookerError::InvalidConfig(format!(
                "pooled funding requires 1 to {MAX_DISBURSERS} disbursers"
            )));
        }
        if self.rounds == 0 || self.rounds > MAX_ROUNDS {
            return Err(CookerError::InvalidConfig(format!(
                "pooled funding requires 1 to {MAX_ROUNDS} rounds"
            )));
        }
        let rounds = usize::try_from(self.rounds).map_err(|_| {
            CookerError::InvalidConfig("funding rounds do not fit usize".to_owned())
        })?;
        if self.top_ups_per_account == 0 || self.top_ups_per_account > rounds {
            return Err(CookerError::InvalidConfig(
                "pooled funding requires 1 to rounds top-ups per account".to_owned(),
            ));
        }
        if self.denomination_lamports == 0 {
            return Err(CookerError::InvalidConfig(
                "pooled funding requires a positive denomination".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Payer assignment strategy used to build a schedule.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FundingScheme {
    /// Every account is paid only by the wallet of its own operator.
    DedicatedPerOperator,
    /// Round participants are shuffled together before shared disbursers are dealt out.
    PooledMixedRounds,
    /// Shared disbursers each pay one operator's whole batch, which is a measured control
    /// rather than a mitigation.
    PooledPerOperatorRounds,
}

/// One account and the operator that owns it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperatorAccount {
    /// Stable account identifier.
    pub account: AgentId,
    /// Operator label, read only by operator-grouped schemes.
    pub operator: String,
}

/// One scheduled uniform-denomination transfer.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct FundingTopUp {
    /// Batching round that carries the transfer.
    pub round: u32,
    /// Index of the paying wallet.
    pub funder: usize,
    /// Funded account.
    pub account: AgentId,
    /// Transferred lamports.
    pub lamports: u64,
}

/// Complete deterministic funding schedule.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FundingPlan {
    scheme: FundingScheme,
    config: PooledFundingConfig,
    by_account: BTreeMap<AgentId, Vec<FundingTopUp>>,
}

impl FundingPlan {
    /// Build a pooled schedule whose round batches are mixed across operators.
    ///
    /// Account identifiers are the only per-account input, so no operator grouping can reach
    /// the payer assignment.
    pub fn pooled_mixed_rounds(
        accounts: &[AgentId],
        config: PooledFundingConfig,
        seed: &[u8; 32],
    ) -> Result<Self, CookerError> {
        let participation = participation(accounts, config, seed)?;
        let mut by_account = empty_history(&participation);
        for round in 0..config.rounds {
            let mut participants = round_participants(&participation, round);
            let mut rng = schedule_rng(seed, "mixed-round", &round.to_le_bytes());
            participants.shuffle(&mut rng);
            for (position, account) in participants.into_iter().enumerate() {
                record(
                    &mut by_account,
                    account,
                    round,
                    position % config.disbursers,
                    config.denomination_lamports,
                );
            }
        }
        Ok(Self {
            scheme: FundingScheme::PooledMixedRounds,
            config,
            by_account,
        })
    }

    /// Build a pooled schedule that batches each operator separately.
    ///
    /// This keeps the shared disburser set and the uniform denomination but lets batch
    /// composition follow operator ownership, which is the control condition.
    pub fn pooled_per_operator_rounds(
        accounts: &[OperatorAccount],
        config: PooledFundingConfig,
        seed: &[u8; 32],
    ) -> Result<Self, CookerError> {
        let operators = operator_index(accounts);
        let identifiers = identifiers(accounts);
        let participation = participation(&identifiers, config, seed)?;
        let mut by_account = empty_history(&participation);
        for round in 0..config.rounds {
            let mut batches: BTreeMap<&str, Vec<AgentId>> = BTreeMap::new();
            for account in round_participants(&participation, round) {
                if let Some(operator) = operators.get(&account) {
                    batches.entry(operator.as_str()).or_default().push(account);
                }
            }
            for (operator, participants) in batches {
                let mut rng = schedule_rng(seed, "operator-round", &label(operator, round));
                let funder = rng.gen_range(0..config.disbursers);
                for account in participants {
                    record(
                        &mut by_account,
                        account,
                        round,
                        funder,
                        config.denomination_lamports,
                    );
                }
            }
        }
        Ok(Self {
            scheme: FundingScheme::PooledPerOperatorRounds,
            config,
            by_account,
        })
    }

    /// Build the baseline schedule in which each operator pays its own accounts.
    ///
    /// Payer indexes address one wallet per operator, so the shared disburser count is not
    /// used by this scheme.
    pub fn dedicated_per_operator(
        accounts: &[OperatorAccount],
        config: PooledFundingConfig,
        seed: &[u8; 32],
    ) -> Result<Self, CookerError> {
        let operators = operator_index(accounts);
        let ordered: BTreeSet<&str> = operators.values().map(String::as_str).collect();
        let wallets: BTreeMap<&str, usize> = ordered
            .into_iter()
            .enumerate()
            .map(|(index, operator)| (operator, index))
            .collect();
        let identifiers = identifiers(accounts);
        let participation = participation(&identifiers, config, seed)?;
        let mut by_account = empty_history(&participation);
        for (account, rounds) in &participation {
            let Some(funder) = operators
                .get(account)
                .and_then(|operator| wallets.get(operator.as_str()))
            else {
                continue;
            };
            for round in rounds {
                record(
                    &mut by_account,
                    *account,
                    *round,
                    *funder,
                    config.denomination_lamports,
                );
            }
        }
        Ok(Self {
            scheme: FundingScheme::DedicatedPerOperator,
            config,
            by_account,
        })
    }

    /// Payer assignment strategy used to build the schedule.
    #[must_use]
    pub const fn scheme(&self) -> FundingScheme {
        self.scheme
    }

    /// Parameters used to build the schedule.
    #[must_use]
    pub const fn config(&self) -> PooledFundingConfig {
        self.config
    }

    /// Scheduled transfers for one account, ordered by round.
    #[must_use]
    pub fn account_history(&self, account: AgentId) -> &[FundingTopUp] {
        self.by_account
            .get(&account)
            .map_or(&[] as &[FundingTopUp], Vec::as_slice)
    }

    /// Most recent transfer at or before a round.
    #[must_use]
    pub fn latest_top_up(&self, account: AgentId, round: u32) -> Option<&FundingTopUp> {
        self.account_history(account)
            .iter()
            .rfind(|top_up| top_up.round <= round)
    }

    /// Distinct payers observed by one account.
    #[must_use]
    pub fn funders_for(&self, account: AgentId) -> BTreeSet<usize> {
        self.account_history(account)
            .iter()
            .map(|top_up| top_up.funder)
            .collect()
    }

    /// Every scheduled transfer, ordered by round, payer, and account.
    #[must_use]
    pub fn ordered_top_ups(&self) -> Vec<FundingTopUp> {
        let mut output: Vec<_> = self.by_account.values().flatten().copied().collect();
        output.sort_unstable();
        output
    }

    /// Number of scheduled transfers.
    #[must_use]
    pub fn transfers(&self) -> usize {
        self.by_account.values().map(Vec::len).sum()
    }

    /// Total lamports the schedule moves.
    #[must_use]
    pub fn total_lamports(&self) -> u128 {
        self.by_account
            .values()
            .flatten()
            .map(|top_up| u128::from(top_up.lamports))
            .sum()
    }

    /// Recipient count of every round and payer batch.
    #[must_use]
    pub fn batch_recipients(&self) -> BTreeMap<(u32, usize), usize> {
        let mut output = BTreeMap::new();
        for top_up in self.by_account.values().flatten() {
            *output.entry((top_up.round, top_up.funder)).or_insert(0) += 1;
        }
        output
    }
}

fn participation(
    accounts: &[AgentId],
    config: PooledFundingConfig,
    seed: &[u8; 32],
) -> Result<BTreeMap<AgentId, BTreeSet<u32>>, CookerError> {
    config.validate()?;
    if accounts.is_empty() {
        return Err(CookerError::InvalidConfig(
            "funding schedule requires at least one account".to_owned(),
        ));
    }
    let horizon = usize::try_from(config.rounds.saturating_sub(1))
        .map_err(|_| CookerError::InvalidConfig("funding rounds do not fit usize".to_owned()))?;
    let extra = config.top_ups_per_account.saturating_sub(1).min(horizon);
    let mut output: BTreeMap<AgentId, BTreeSet<u32>> = BTreeMap::new();
    for account in accounts {
        if output.contains_key(account) {
            return Err(CookerError::InvalidConfig(
                "funding schedule requires unique accounts".to_owned(),
            ));
        }
        let mut rounds = BTreeSet::from([0_u32]);
        let mut rng = schedule_rng(seed, "participation", account.0.as_bytes());
        for index in rand::seq::index::sample(&mut rng, horizon, extra) {
            let round = u32::try_from(index.saturating_add(1)).map_err(|_| {
                CookerError::InvalidConfig("funding round does not fit u32".to_owned())
            })?;
            rounds.insert(round);
        }
        output.insert(*account, rounds);
    }
    Ok(output)
}

fn empty_history(
    participation: &BTreeMap<AgentId, BTreeSet<u32>>,
) -> BTreeMap<AgentId, Vec<FundingTopUp>> {
    participation
        .keys()
        .map(|account| (*account, Vec::new()))
        .collect()
}

fn round_participants(
    participation: &BTreeMap<AgentId, BTreeSet<u32>>,
    round: u32,
) -> Vec<AgentId> {
    participation
        .iter()
        .filter(|(_, rounds)| rounds.contains(&round))
        .map(|(account, _)| *account)
        .collect()
}

fn record(
    by_account: &mut BTreeMap<AgentId, Vec<FundingTopUp>>,
    account: AgentId,
    round: u32,
    funder: usize,
    lamports: u64,
) {
    by_account.entry(account).or_default().push(FundingTopUp {
        round,
        funder,
        account,
        lamports,
    });
}

fn operator_index(accounts: &[OperatorAccount]) -> BTreeMap<AgentId, String> {
    accounts
        .iter()
        .map(|entry| (entry.account, entry.operator.clone()))
        .collect()
}

fn identifiers(accounts: &[OperatorAccount]) -> Vec<AgentId> {
    accounts.iter().map(|entry| entry.account).collect()
}

fn label(operator: &str, round: u32) -> Vec<u8> {
    let mut output = Vec::with_capacity(operator.len() + 12);
    output.extend_from_slice(&(operator.len() as u64).to_le_bytes());
    output.extend_from_slice(operator.as_bytes());
    output.extend_from_slice(&round.to_le_bytes());
    output
}

fn schedule_rng(seed: &[u8; 32], label: &str, discriminator: &[u8]) -> ChaCha12Rng {
    let mut hasher = blake3::Hasher::new();
    hasher.update(seed);
    hasher.update(SCHEDULE_VERSION.as_bytes());
    hasher.update(label.as_bytes());
    hasher.update(discriminator);
    ChaCha12Rng::from_seed(*hasher.finalize().as_bytes())
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;

    fn config() -> PooledFundingConfig {
        PooledFundingConfig {
            disbursers: 8,
            rounds: 30,
            top_ups_per_account: 4,
            denomination_lamports: 600_000_000,
        }
    }

    fn roster(operators: usize, per_operator: usize) -> Vec<OperatorAccount> {
        let mut output = Vec::new();
        for operator in 0..operators {
            for index in 0..per_operator {
                output.push(OperatorAccount {
                    account: AgentId(Uuid::new_v5(
                        &Uuid::NAMESPACE_OID,
                        format!("funding-test-{operator}-{index}").as_bytes(),
                    )),
                    operator: format!("operator-{operator:03}"),
                });
            }
        }
        output
    }

    #[test]
    fn mixed_schedule_is_stable_under_account_order() -> Result<(), CookerError> {
        let accounts = identifiers(&roster(4, 5));
        let mut reversed = accounts.clone();
        reversed.reverse();
        let left = FundingPlan::pooled_mixed_rounds(&accounts, config(), &[3; 32])?;
        let right = FundingPlan::pooled_mixed_rounds(&reversed, config(), &[3; 32])?;
        assert_eq!(left, right);
        Ok(())
    }

    #[test]
    fn mixed_schedule_cannot_depend_on_operator_labels() -> Result<(), CookerError> {
        let accounts = roster(4, 5);
        let relabelled: Vec<_> = accounts
            .iter()
            .enumerate()
            .map(|(index, entry)| OperatorAccount {
                account: entry.account,
                operator: format!("relabelled-{index:03}"),
            })
            .collect();
        let left = FundingPlan::pooled_mixed_rounds(&identifiers(&accounts), config(), &[5; 32])?;
        let right =
            FundingPlan::pooled_mixed_rounds(&identifiers(&relabelled), config(), &[5; 32])?;
        assert_eq!(left, right);
        Ok(())
    }

    #[test]
    fn every_scheme_shares_one_participation_schedule() -> Result<(), CookerError> {
        let accounts = roster(6, 5);
        let mixed = FundingPlan::pooled_mixed_rounds(&identifiers(&accounts), config(), &[9; 32])?;
        let grouped = FundingPlan::pooled_per_operator_rounds(&accounts, config(), &[9; 32])?;
        let dedicated = FundingPlan::dedicated_per_operator(&accounts, config(), &[9; 32])?;
        for entry in &accounts {
            let rounds = |plan: &FundingPlan| {
                plan.account_history(entry.account)
                    .iter()
                    .map(|top_up| top_up.round)
                    .collect::<Vec<_>>()
            };
            assert_eq!(rounds(&mixed), rounds(&grouped));
            assert_eq!(rounds(&mixed), rounds(&dedicated));
            assert_eq!(rounds(&mixed).len(), config().top_ups_per_account);
        }
        Ok(())
    }

    #[test]
    fn every_account_is_provisioned_in_the_first_round() -> Result<(), CookerError> {
        let accounts = roster(3, 4);
        let plan = FundingPlan::pooled_mixed_rounds(&identifiers(&accounts), config(), &[1; 32])?;
        for entry in &accounts {
            assert_eq!(
                plan.account_history(entry.account)
                    .first()
                    .map(|top_up| top_up.round),
                Some(0)
            );
        }
        Ok(())
    }

    #[test]
    fn every_transfer_uses_the_declared_denomination() -> Result<(), CookerError> {
        let accounts = roster(3, 4);
        let plan = FundingPlan::pooled_mixed_rounds(&identifiers(&accounts), config(), &[2; 32])?;
        assert!(
            plan.ordered_top_ups()
                .iter()
                .all(|top_up| top_up.lamports == config().denomination_lamports)
        );
        assert_eq!(plan.transfers(), 12 * config().top_ups_per_account);
        assert_eq!(
            plan.total_lamports(),
            u128::from(config().denomination_lamports) * 48
        );
        Ok(())
    }

    #[test]
    fn grouped_rounds_pay_each_operator_from_one_wallet() -> Result<(), CookerError> {
        let accounts = roster(6, 6);
        let operators = operator_index(&accounts);
        let grouped = FundingPlan::pooled_per_operator_rounds(&accounts, config(), &[4; 32])?;
        let mixed = FundingPlan::pooled_mixed_rounds(&identifiers(&accounts), config(), &[4; 32])?;
        let operator_round_funders = |plan: &FundingPlan| {
            let mut output: BTreeMap<(u32, String), BTreeSet<usize>> = BTreeMap::new();
            for top_up in plan.ordered_top_ups() {
                if let Some(operator) = operators.get(&top_up.account) {
                    output
                        .entry((top_up.round, operator.clone()))
                        .or_default()
                        .insert(top_up.funder);
                }
            }
            output
        };
        assert!(
            operator_round_funders(&grouped)
                .values()
                .all(|funders| funders.len() == 1)
        );
        assert!(
            operator_round_funders(&mixed)
                .values()
                .any(|funders| funders.len() > 1)
        );
        let shared: BTreeSet<_> = grouped
            .ordered_top_ups()
            .iter()
            .map(|top_up| top_up.funder)
            .collect();
        assert!(shared.len() > 1, "the control keeps a shared disburser set");
        Ok(())
    }

    #[test]
    fn dedicated_schedules_use_one_wallet_per_operator() -> Result<(), CookerError> {
        let accounts = roster(5, 4);
        let plan = FundingPlan::dedicated_per_operator(&accounts, config(), &[7; 32])?;
        let mut wallets: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
        for entry in &accounts {
            wallets
                .entry(entry.operator.clone())
                .or_default()
                .extend(plan.funders_for(entry.account));
        }
        assert_eq!(wallets.len(), 5);
        assert!(wallets.values().all(|funders| funders.len() == 1));
        let distinct: BTreeSet<_> = wallets.values().flatten().copied().collect();
        assert_eq!(distinct.len(), 5);
        Ok(())
    }

    #[test]
    fn latest_top_up_tracks_the_current_payer() -> Result<(), CookerError> {
        let accounts = roster(2, 3);
        let plan = FundingPlan::pooled_mixed_rounds(&identifiers(&accounts), config(), &[8; 32])?;
        for entry in &accounts {
            let history = plan.account_history(entry.account);
            for round in 0..config().rounds {
                let expected = history.iter().rfind(|top_up| top_up.round <= round);
                assert_eq!(plan.latest_top_up(entry.account, round), expected);
            }
        }
        Ok(())
    }

    #[test]
    fn invalid_parameters_are_rejected() {
        let accounts = identifiers(&roster(2, 2));
        let cases = [
            PooledFundingConfig {
                disbursers: 0,
                ..config()
            },
            PooledFundingConfig {
                rounds: 0,
                ..config()
            },
            PooledFundingConfig {
                top_ups_per_account: 0,
                ..config()
            },
            PooledFundingConfig {
                top_ups_per_account: 31,
                ..config()
            },
            PooledFundingConfig {
                denomination_lamports: 0,
                ..config()
            },
        ];
        for case in cases {
            assert!(FundingPlan::pooled_mixed_rounds(&accounts, case, &[0; 32]).is_err());
        }
        assert!(FundingPlan::pooled_mixed_rounds(&[], config(), &[0; 32]).is_err());
        let duplicate = [accounts[0], accounts[0]];
        assert!(FundingPlan::pooled_mixed_rounds(&duplicate, config(), &[0; 32]).is_err());
    }
}
