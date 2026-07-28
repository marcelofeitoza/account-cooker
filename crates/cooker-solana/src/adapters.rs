//! Native SOL and classic SPL Token action adapters.

use std::{collections::BTreeMap, sync::Arc};

use cooker_core::{
    ActionAdapter, ActionPayload, AdapterContext, ChainReceipt, ConfirmationStatus, CookerError,
    PlannedAction, PreparedAction, StateExpectation,
};
use solana_program_pack::Pack;
use solana_pubkey::Pubkey;
use solana_system_interface::instruction as system_instruction;
use spl_associated_token_account_interface::{
    address::get_associated_token_address, instruction::create_associated_token_account_idempotent,
};
use spl_token_interface::{
    instruction::transfer_checked,
    state::{Account as TokenAccount, Mint},
};

use crate::{
    AccountInfo, LocalKeypair, SolanaGateway,
    adapter_support::{
        attribute_u64, confirmed_record, find_expectation, observe_until_terminal, parse_pubkey,
        prepared_action, transaction_native_balances, validate_context, validate_prepared,
    },
    build_signed_transaction,
};
const NATIVE_DESTINATION_KIND: &str = "native_destination_balance_delta";
const NATIVE_SOURCE_KIND: &str = "native_source_balance_delta";
const SPL_DESTINATION_KIND: &str = "spl_destination_balance_delta";
const SPL_NATIVE_OVERHEAD_KIND: &str = "spl_payer_native_overhead";
const SPL_SOURCE_KIND: &str = "spl_source_balance_delta";
const SPL_TRANSACTION_METADATA_KIND: &str = "spl_transaction_token_metadata";

/// Adapter that builds one-signer native SOL transfers for an identity-verified gateway.
#[derive(Clone, Debug)]
pub struct NativeTransferAdapter {
    gateway: Arc<SolanaGateway>,
    signer: Arc<LocalKeypair>,
}

impl NativeTransferAdapter {
    /// Create an adapter from an already identity-verified gateway and local signer.
    #[must_use]
    pub const fn new(gateway: Arc<SolanaGateway>, signer: Arc<LocalKeypair>) -> Self {
        Self { gateway, signer }
    }
}

/// Adapter that builds classic SPL Token transfers between associated accounts.
#[derive(Clone, Debug)]
pub struct SplTransferAdapter {
    gateway: Arc<SolanaGateway>,
    signer: Arc<LocalKeypair>,
}

impl SplTransferAdapter {
    /// Create an adapter from an already identity-verified gateway and local signer.
    #[must_use]
    pub const fn new(gateway: Arc<SolanaGateway>, signer: Arc<LocalKeypair>) -> Self {
        Self { gateway, signer }
    }
}

#[async_trait::async_trait]
impl ActionAdapter for NativeTransferAdapter {
    fn supports(&self, payload: &ActionPayload) -> bool {
        matches!(payload, ActionPayload::NativeTransfer { .. })
    }

    async fn prepare(
        &self,
        context: &AdapterContext,
        action: &PlannedAction,
    ) -> Result<PreparedAction, CookerError> {
        validate_context(context, &self.gateway, &self.signer)?;
        let ActionPayload::NativeTransfer {
            destination,
            lamports,
        } = &action.payload
        else {
            return Err(unsupported_action("native SOL transfer"));
        };
        if *lamports == 0 {
            return Err(CookerError::Policy(
                "native transfer amount must be positive".to_owned(),
            ));
        }
        let destination = parse_pubkey(destination, "native destination")?;
        let source = self.signer.pubkey();
        if destination == source {
            return Err(CookerError::Policy(
                "native self-transfer is not a supported state change".to_owned(),
            ));
        }

        let required = lamports
            .checked_add(action.max_fee_lamports)
            .ok_or_else(|| CookerError::Policy("native transfer spend overflowed".to_owned()))?;
        let (source_before, destination_before) = tokio::try_join!(
            self.gateway.balance(&source),
            self.gateway.balance(&destination)
        )?;
        if source_before < required {
            return Err(CookerError::Policy(format!(
                "native balance {source_before} cannot cover transfer plus fee ceiling {required}"
            )));
        }

        let latest = self.gateway.latest_blockhash().await?;
        let instruction = system_instruction::transfer(&source, &destination, *lamports);
        let signed = build_signed_transaction(&[instruction], &self.signer, latest)?;
        let expectations = vec![
            native_expectation(
                NATIVE_SOURCE_KIND,
                source,
                source_before,
                -i128::from(*lamports),
                *lamports,
                action.max_fee_lamports,
            ),
            native_expectation(
                NATIVE_DESTINATION_KIND,
                destination,
                destination_before,
                i128::from(*lamports),
                *lamports,
                action.max_fee_lamports,
            ),
        ];
        Ok(prepared_action(action, signed, expectations))
    }

    async fn observe(
        &self,
        context: &AdapterContext,
        action: &PlannedAction,
        prepared: &PreparedAction,
    ) -> Result<ChainReceipt, CookerError> {
        validate_context(context, &self.gateway, &self.signer)?;
        validate_prepared(action, prepared, &self.signer, "native")?;
        let ActionPayload::NativeTransfer {
            destination,
            lamports,
        } = &action.payload
        else {
            return Err(unsupported_action("native SOL transfer"));
        };
        let destination = parse_pubkey(destination, "native destination")?;
        let source = self.signer.pubkey();
        let mut receipt = observe_until_terminal(&self.gateway, context, prepared).await?;
        if !is_success_status(receipt.status) {
            return Ok(receipt);
        }

        let record = confirmed_record(&self.gateway, prepared, "native").await?;
        let source_expected = find_expectation(prepared, NATIVE_SOURCE_KIND, &source)?;
        let destination_expected =
            find_expectation(prepared, NATIVE_DESTINATION_KIND, &destination)?;
        let (source_before, source_after) =
            transaction_native_balances(&record, &source, "native source")?;
        let (destination_before, destination_after) =
            transaction_native_balances(&record, &destination, "native destination")?;
        let source_delta = i128::from(source_after) - i128::from(source_before);
        let destination_delta = i128::from(destination_after) - i128::from(destination_before);
        let exact_debit = lamports
            .checked_add(record.fee)
            .ok_or_else(|| CookerError::Codec("native transaction debit overflowed".to_owned()))?;
        let source_met = source_delta == -i128::from(exact_debit);
        let destination_met = destination_delta == i128::from(*lamports);
        let fee_met = record.fee <= action.max_fee_lamports;

        receipt.observations = vec![
            observed_transaction_expectation(
                source_expected,
                source_before,
                source_after,
                source_delta,
                source_met,
            ),
            observed_transaction_expectation(
                destination_expected,
                destination_before,
                destination_after,
                destination_delta,
                destination_met,
            ),
            fee_observation(source, record.fee, action.max_fee_lamports, fee_met),
        ];
        receipt.postconditions_met = source_met && destination_met && fee_met;
        if !receipt.postconditions_met {
            receipt.error = Some("native transfer postconditions were not proven".to_owned());
        }
        Ok(receipt)
    }
}

#[async_trait::async_trait]
#[allow(
    clippy::too_many_lines,
    reason = "SPL preparation keeps ordered account, owner, amount, and instruction validation explicit"
)]
impl ActionAdapter for SplTransferAdapter {
    fn supports(&self, payload: &ActionPayload) -> bool {
        matches!(payload, ActionPayload::SplTransfer { .. })
    }

    async fn prepare(
        &self,
        context: &AdapterContext,
        action: &PlannedAction,
    ) -> Result<PreparedAction, CookerError> {
        validate_context(context, &self.gateway, &self.signer)?;
        let ActionPayload::SplTransfer {
            mint,
            destination_owner,
            amount,
        } = &action.payload
        else {
            return Err(unsupported_action("SPL token transfer"));
        };
        if *amount == 0 {
            return Err(CookerError::Policy(
                "SPL transfer amount must be positive".to_owned(),
            ));
        }

        let mint = parse_pubkey(mint, "SPL mint")?;
        let destination_owner = parse_pubkey(destination_owner, "SPL destination owner")?;
        let signer = self.signer.pubkey();
        if destination_owner == signer {
            return Err(CookerError::Policy(
                "SPL self-transfer is not a supported state change".to_owned(),
            ));
        }
        let source = get_associated_token_address(&signer, &mint);
        let destination = get_associated_token_address(&destination_owner, &mint);
        let token_program = spl_token_interface::id();

        let (mint_account, source_account) = tokio::try_join!(
            required_account(&self.gateway, &mint, "SPL mint"),
            required_account(&self.gateway, &source, "source associated token account")
        )?;
        let (destination_account, native_balance) = tokio::try_join!(
            async {
                self.gateway
                    .account(&destination)
                    .await
                    .map_err(CookerError::from)
            },
            async {
                self.gateway
                    .balance(&signer)
                    .await
                    .map_err(CookerError::from)
            }
        )?;
        let destination_creation_rent = if destination_account.is_none() {
            self.gateway
                .minimum_balance_for_rent_exemption(TokenAccount::LEN)
                .await?
        } else {
            0
        };
        if destination_creation_rent > action.max_account_creation_lamports {
            return Err(CookerError::Policy(format!(
                "destination ATA rent {destination_creation_rent} exceeds account-creation cap {}",
                action.max_account_creation_lamports
            )));
        }
        let required_native = action
            .max_fee_lamports
            .checked_add(destination_creation_rent)
            .ok_or_else(|| CookerError::Policy("SPL native overhead overflowed".to_owned()))?;
        if native_balance < required_native {
            return Err(CookerError::Policy(format!(
                "native balance {native_balance} is below fee plus ATA-rent requirement {required_native}"
            )));
        }
        validate_token_program_owner(&mint_account, &token_program, "SPL mint")?;
        validate_token_program_owner(&source_account, &token_program, "source token account")?;
        let mint_state = unpack_mint(&mint_account)?;
        let source_state = unpack_token_account(&source_account, &mint, &signer, "source")?;
        if source_state.amount < *amount {
            return Err(CookerError::Policy(format!(
                "source token balance {} cannot cover transfer amount {amount}",
                source_state.amount
            )));
        }

        let destination_before = if let Some(account) = destination_account.as_ref() {
            validate_token_program_owner(account, &token_program, "destination token account")?;
            unpack_token_account(account, &mint, &destination_owner, "destination")?.amount
        } else {
            0
        };
        let mut instructions = Vec::with_capacity(2);
        if destination_account.is_none() {
            instructions.push(create_associated_token_account_idempotent(
                &signer,
                &destination_owner,
                &mint,
                &token_program,
            ));
        }
        instructions.push(
            transfer_checked(
                &token_program,
                &source,
                &mint,
                &destination,
                &signer,
                &[],
                *amount,
                mint_state.decimals,
            )
            .map_err(|error| {
                CookerError::Codec(format!("cannot build checked SPL transfer: {error}"))
            })?,
        );

        let latest = self.gateway.latest_blockhash().await?;
        let wire_transaction = build_signed_transaction(&instructions, &self.signer, latest)?;
        let expectations = vec![
            spl_expectation(
                SPL_SOURCE_KIND,
                source,
                mint,
                signer,
                source_state.amount,
                -i128::from(*amount),
                mint_state.decimals,
            ),
            spl_expectation(
                SPL_DESTINATION_KIND,
                destination,
                mint,
                destination_owner,
                destination_before,
                i128::from(*amount),
                mint_state.decimals,
            ),
            spl_native_overhead_expectation(
                signer,
                native_balance,
                destination_creation_rent,
                action.max_account_creation_lamports,
            ),
        ];
        Ok(prepared_action(action, wire_transaction, expectations))
    }

    async fn observe(
        &self,
        context: &AdapterContext,
        action: &PlannedAction,
        prepared: &PreparedAction,
    ) -> Result<ChainReceipt, CookerError> {
        validate_context(context, &self.gateway, &self.signer)?;
        validate_prepared(action, prepared, &self.signer, "SPL")?;
        let ActionPayload::SplTransfer {
            mint,
            destination_owner,
            amount: _,
        } = &action.payload
        else {
            return Err(unsupported_action("SPL token transfer"));
        };
        let mint = parse_pubkey(mint, "SPL mint")?;
        let destination_owner = parse_pubkey(destination_owner, "SPL destination owner")?;
        let signer = self.signer.pubkey();
        let source = get_associated_token_address(&signer, &mint);
        let destination = get_associated_token_address(&destination_owner, &mint);
        let mut receipt = observe_until_terminal(&self.gateway, context, prepared).await?;
        if !is_success_status(receipt.status) {
            return Ok(receipt);
        }

        let record = confirmed_record(&self.gateway, prepared, "SPL").await?;
        let source_expected = find_expectation(prepared, SPL_SOURCE_KIND, &source)?;
        let destination_expected = find_expectation(prepared, SPL_DESTINATION_KIND, &destination)?;
        let native_expected = find_expectation(prepared, SPL_NATIVE_OVERHEAD_KIND, &signer)?;
        let expected_creation_rent = attribute_u64(native_expected, "creation_rent_lamports")?;
        let decimals = attribute_u8(source_expected, "decimals")?;
        let source_pre = required_token_balance(
            record.pre_token_balance(&source, &mint),
            "source pre-balance",
        )?;
        let source_post = required_token_balance(
            record.post_token_balance(&source, &mint),
            "source post-balance",
        )?;
        let destination_pre = record.pre_token_balance(&destination, &mint);
        let destination_post = required_token_balance(
            record.post_token_balance(&destination, &mint),
            "destination post-balance",
        )?;
        let source_before = source_pre.amount;
        let source_after = source_post.amount;
        let destination_before = match destination_pre {
            Some(balance) => balance.amount,
            None if expected_creation_rent > 0 => 0,
            None => {
                return Err(CookerError::Codec(
                    "transaction metadata omitted existing destination pre-balance".to_owned(),
                ));
            }
        };
        let destination_after = destination_post.amount;
        let source_delta = i128::from(source_after) - i128::from(source_before);
        let destination_delta = i128::from(destination_after) - i128::from(destination_before);
        let source_met = source_expected.expected_delta == Some(source_delta);
        let destination_met = destination_expected.expected_delta == Some(destination_delta);
        let fee_met = record.fee <= action.max_fee_lamports;
        let (native_before, native_after) =
            transaction_native_balances(&record, &signer, "SPL payer")?;
        let (destination_native_before, destination_native_after) =
            transaction_native_balances(&record, &destination, "SPL destination ATA")?;
        let native_debit = native_before.saturating_sub(native_after);
        let actual_creation_rent = native_debit.checked_sub(record.fee);
        let destination_native_delta =
            i128::from(destination_native_after) - i128::from(destination_native_before);
        let native_overhead_met = native_after <= native_before
            && actual_creation_rent == Some(expected_creation_rent)
            && destination_native_delta == i128::from(expected_creation_rent);
        let (metadata_observation, metadata_met) = spl_transaction_metadata_observation(
            &record,
            &mint,
            &source,
            &signer,
            &destination,
            &destination_owner,
            decimals,
            expected_creation_rent > 0,
        );

        receipt.observations = vec![
            observed_transaction_expectation(
                source_expected,
                source_before,
                source_after,
                source_delta,
                source_met,
            ),
            observed_transaction_expectation(
                destination_expected,
                destination_before,
                destination_after,
                destination_delta,
                destination_met,
            ),
            fee_observation(signer, record.fee, action.max_fee_lamports, fee_met),
            observed_native_overhead(
                native_expected,
                native_before,
                native_after,
                native_debit,
                record.fee,
                actual_creation_rent,
                destination_native_before,
                destination_native_after,
                native_overhead_met,
            ),
            metadata_observation,
        ];
        receipt.postconditions_met =
            source_met && destination_met && fee_met && native_overhead_met && metadata_met;
        if !receipt.postconditions_met {
            receipt.error = Some("SPL transfer postconditions were not proven".to_owned());
        }
        Ok(receipt)
    }
}

async fn required_account(
    gateway: &SolanaGateway,
    address: &Pubkey,
    label: &str,
) -> Result<AccountInfo, CookerError> {
    gateway
        .account(address)
        .await?
        .ok_or_else(|| CookerError::NotFound(format!("{label} {address}")))
}

fn validate_token_program_owner(
    account: &AccountInfo,
    token_program: &Pubkey,
    label: &str,
) -> Result<(), CookerError> {
    if &account.owner != token_program {
        return Err(CookerError::Policy(format!(
            "{label} is not owned by the classic SPL Token program"
        )));
    }
    Ok(())
}

fn unpack_mint(account: &AccountInfo) -> Result<Mint, CookerError> {
    Mint::unpack(&account.data)
        .map_err(|error| CookerError::Codec(format!("invalid initialized SPL mint: {error}")))
}

fn unpack_token_account(
    account: &AccountInfo,
    mint: &Pubkey,
    owner: &Pubkey,
    label: &str,
) -> Result<TokenAccount, CookerError> {
    let token = TokenAccount::unpack(&account.data).map_err(|error| {
        CookerError::Codec(format!(
            "invalid initialized {label} token account: {error}"
        ))
    })?;
    if &token.mint != mint || &token.owner != owner {
        return Err(CookerError::Policy(format!(
            "{label} associated token account has unexpected mint or owner"
        )));
    }
    if token.is_frozen() {
        return Err(CookerError::Policy(format!(
            "{label} associated token account is frozen"
        )));
    }
    Ok(token)
}

fn native_expectation(
    kind: &str,
    account: Pubkey,
    before: u64,
    expected_delta: i128,
    lamports: u64,
    max_fee_lamports: u64,
) -> StateExpectation {
    StateExpectation {
        kind: kind.to_owned(),
        account: account.to_string(),
        expected_delta: Some(expected_delta),
        attributes: BTreeMap::from([
            ("before".to_owned(), before.to_string()),
            ("transfer_lamports".to_owned(), lamports.to_string()),
            ("max_fee_lamports".to_owned(), max_fee_lamports.to_string()),
        ]),
    }
}

fn spl_expectation(
    kind: &str,
    account: Pubkey,
    mint: Pubkey,
    owner: Pubkey,
    before: u64,
    expected_delta: i128,
    decimals: u8,
) -> StateExpectation {
    StateExpectation {
        kind: kind.to_owned(),
        account: account.to_string(),
        expected_delta: Some(expected_delta),
        attributes: BTreeMap::from([
            ("before".to_owned(), before.to_string()),
            ("decimals".to_owned(), decimals.to_string()),
            ("mint".to_owned(), mint.to_string()),
            ("owner".to_owned(), owner.to_string()),
        ]),
    }
}

fn spl_native_overhead_expectation(
    payer: Pubkey,
    before: u64,
    creation_rent_lamports: u64,
    max_account_creation_lamports: u64,
) -> StateExpectation {
    StateExpectation {
        kind: SPL_NATIVE_OVERHEAD_KIND.to_owned(),
        account: payer.to_string(),
        expected_delta: None,
        attributes: BTreeMap::from([
            ("before".to_owned(), before.to_string()),
            (
                "creation_rent_lamports".to_owned(),
                creation_rent_lamports.to_string(),
            ),
            (
                "max_account_creation_lamports".to_owned(),
                max_account_creation_lamports.to_string(),
            ),
        ]),
    }
}

fn attribute_u8(expectation: &StateExpectation, key: &str) -> Result<u8, CookerError> {
    expectation
        .attributes
        .get(key)
        .ok_or_else(|| CookerError::Codec(format!("expectation omitted {key}")))?
        .parse()
        .map_err(|error| CookerError::Codec(format!("invalid expectation {key}: {error}")))
}

fn observed_expectation(
    expected: &StateExpectation,
    after: u64,
    actual_delta: i128,
    met: bool,
) -> StateExpectation {
    let mut observation = expected.clone();
    observation
        .attributes
        .insert("after".to_owned(), after.to_string());
    observation
        .attributes
        .insert("actual_delta".to_owned(), actual_delta.to_string());
    observation
        .attributes
        .insert("met".to_owned(), met.to_string());
    observation
}

fn observed_transaction_expectation(
    expected: &StateExpectation,
    transaction_before: u64,
    transaction_after: u64,
    actual_delta: i128,
    met: bool,
) -> StateExpectation {
    let mut observation = observed_expectation(expected, transaction_after, actual_delta, met);
    observation.attributes.insert(
        "transaction_before".to_owned(),
        transaction_before.to_string(),
    );
    observation
}

fn fee_observation(payer: Pubkey, actual_fee: u64, max_fee: u64, met: bool) -> StateExpectation {
    StateExpectation {
        kind: "transaction_fee_ceiling".to_owned(),
        account: payer.to_string(),
        expected_delta: None,
        attributes: BTreeMap::from([
            ("actual_fee_lamports".to_owned(), actual_fee.to_string()),
            ("max_fee_lamports".to_owned(), max_fee.to_string()),
            ("met".to_owned(), met.to_string()),
        ]),
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "the native-overhead observation records immutable payer and ATA transaction deltas"
)]
fn observed_native_overhead(
    expected: &StateExpectation,
    transaction_before: u64,
    after: u64,
    native_debit: u64,
    transaction_fee: u64,
    actual_creation_rent: Option<u64>,
    destination_before: u64,
    destination_after: u64,
    met: bool,
) -> StateExpectation {
    let mut observation = expected.clone();
    observation.attributes.insert(
        "transaction_before".to_owned(),
        transaction_before.to_string(),
    );
    observation
        .attributes
        .insert("after".to_owned(), after.to_string());
    observation
        .attributes
        .insert("actual_native_debit".to_owned(), native_debit.to_string());
    observation
        .attributes
        .insert("transaction_fee".to_owned(), transaction_fee.to_string());
    observation.attributes.insert(
        "actual_creation_rent".to_owned(),
        actual_creation_rent.map_or_else(|| "underflow".to_owned(), |rent| rent.to_string()),
    );
    observation.attributes.insert(
        "destination_transaction_before".to_owned(),
        destination_before.to_string(),
    );
    observation.attributes.insert(
        "destination_transaction_after".to_owned(),
        destination_after.to_string(),
    );
    observation
        .attributes
        .insert("met".to_owned(), met.to_string());
    observation
}

#[allow(
    clippy::too_many_arguments,
    reason = "transaction metadata proof compares both token accounts and their expected owners"
)]
fn spl_transaction_metadata_observation(
    record: &crate::TransactionRecord,
    mint: &Pubkey,
    source: &Pubkey,
    source_owner: &Pubkey,
    destination: &Pubkey,
    destination_owner: &Pubkey,
    decimals: u8,
    destination_created: bool,
) -> (StateExpectation, bool) {
    let source_pre = record.pre_token_balance(source, mint);
    let source_post = record.post_token_balance(source, mint);
    let destination_pre = record.pre_token_balance(destination, mint);
    let destination_post = record.post_token_balance(destination, mint);
    let metadata_exposed =
        !record.pre_token_balances.is_empty() && !record.post_token_balances.is_empty();
    let token_program = spl_token_interface::id();
    let source_met = source_pre.is_some_and(|balance| {
        token_record_matches(balance, source_owner, &token_program, decimals)
    }) && source_post.is_some_and(|balance| {
        token_record_matches(balance, source_owner, &token_program, decimals)
    });
    let destination_pre_met = destination_pre.map_or(destination_created, |balance| {
        token_record_matches(balance, destination_owner, &token_program, decimals)
    });
    let destination_post_met = destination_post.is_some_and(|balance| {
        token_record_matches(balance, destination_owner, &token_program, decimals)
    });
    let met = metadata_exposed && source_met && destination_pre_met && destination_post_met;
    (
        StateExpectation {
            kind: SPL_TRANSACTION_METADATA_KIND.to_owned(),
            account: record
                .account_keys
                .first()
                .map_or_else(String::new, ToString::to_string),
            expected_delta: None,
            attributes: BTreeMap::from([
                ("exposed".to_owned(), metadata_exposed.to_string()),
                (
                    "source_pre".to_owned(),
                    source_pre
                        .map_or_else(|| "missing".to_owned(), |value| value.amount.to_string()),
                ),
                (
                    "source_post".to_owned(),
                    source_post
                        .map_or_else(|| "missing".to_owned(), |value| value.amount.to_string()),
                ),
                (
                    "destination_pre".to_owned(),
                    destination_pre
                        .map_or_else(|| "missing".to_owned(), |value| value.amount.to_string()),
                ),
                (
                    "destination_post".to_owned(),
                    destination_post
                        .map_or_else(|| "missing".to_owned(), |value| value.amount.to_string()),
                ),
                ("met".to_owned(), met.to_string()),
            ]),
        },
        met,
    )
}

fn token_record_matches(
    balance: &crate::TokenBalanceRecord,
    owner: &Pubkey,
    token_program: &Pubkey,
    decimals: u8,
) -> bool {
    balance.decimals == decimals
        && balance.owner.is_none_or(|value| value == *owner)
        && balance
            .program_id
            .is_none_or(|value| value == *token_program)
}

fn required_token_balance<'a>(
    balance: Option<&'a crate::TokenBalanceRecord>,
    label: &str,
) -> Result<&'a crate::TokenBalanceRecord, CookerError> {
    balance.ok_or_else(|| CookerError::Codec(format!("transaction metadata omitted {label}")))
}

fn unsupported_action(expected: &str) -> CookerError {
    CookerError::Policy(format!("adapter expected {expected}"))
}

const fn is_success_status(status: ConfirmationStatus) -> bool {
    matches!(
        status,
        ConfirmationStatus::Confirmed | ConfirmationStatus::Finalized
    )
}

#[cfg(test)]
mod tests {
    use solana_hash::Hash;
    use solana_keypair::Keypair;
    use solana_signature::Signature;

    use super::*;
    use crate::LatestBlockhash;

    #[test]
    fn prepared_validation_rejects_signature_substitution() -> Result<(), Box<dyn std::error::Error>>
    {
        let local_signer = LocalKeypair::from_keypair_for_test(Keypair::new());
        let instruction = system_instruction::transfer(
            &local_signer.pubkey(),
            &Pubkey::new_from_array([5; 32]),
            1,
        );
        let wire_transaction = build_signed_transaction(
            &[instruction],
            &local_signer,
            LatestBlockhash {
                context_slot: 1,
                blockhash: Hash::new_from_array([4; 32]),
                last_valid_block_height: 10,
            },
        )?;
        let action = test_action(ActionPayload::NativeTransfer {
            destination: Pubkey::new_from_array([5; 32]).to_string(),
            lamports: 1,
        });
        let mut prepared = prepared_action(&action, wire_transaction, Vec::new());
        prepared.signature = Signature::default().to_string();

        assert!(validate_prepared(&action, &prepared, &local_signer, "native").is_err());
        Ok(())
    }

    fn test_action(payload: ActionPayload) -> PlannedAction {
        use chrono::Utc;
        use cooker_core::{ActionId, AgentId, RunId};

        let run_id = RunId::new();
        let agent_id = AgentId::new();
        PlannedAction {
            id: ActionId::derive(run_id, agent_id, 0, "test"),
            run_id,
            agent_id,
            sequence: 0,
            model_version: "test".to_owned(),
            scheduled_at: Utc::now(),
            payload,
            max_fee_lamports: 5_000,
            max_account_creation_lamports: 0,
            created_at: Utc::now(),
        }
    }
}
