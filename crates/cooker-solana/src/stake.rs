//! Native stake lifecycle adapter for deterministic Surfpool acceptance.

use std::{collections::BTreeMap, str::FromStr, sync::Arc, time::Duration};

use cooker_core::{
    ActionAdapter, ActionId, ActionPayload, AdapterContext, ChainGateway, ChainReceipt,
    ConfirmationStatus, CookerError, PlannedAction, PreparedAction, StakeOperation,
    StateExpectation,
};
use solana_pubkey::Pubkey;
use solana_signature::Signature;
use solana_stake_interface::{
    instruction as stake_instruction,
    program::ID as STAKE_PROGRAM_ID,
    state::{Authorized, Lockup, StakeStateV2},
};
use solana_transaction::Transaction;
use tokio::time::{Instant, sleep};

use crate::{
    AccountInfo, LocalKeypair, SignedWireTransaction, SolanaGateway, TransactionRecord,
    build_signed_transaction,
};

const OBSERVATION_POLL_INTERVAL: Duration = Duration::from_millis(200);
const POSITION_KIND: &str = "native_stake_position";
const PAYER_KIND: &str = "native_stake_payer_delta";
const FEE_KIND: &str = "transaction_fee";
const STAKE_SEED_PREFIX: &str = "cooker-stake-";

/// Adapter for create/delegate, deactivate, and withdraw of native stake on Surfpool.
#[derive(Clone, Debug)]
pub struct NativeStakeAdapter {
    gateway: Arc<SolanaGateway>,
    signer: Arc<LocalKeypair>,
    vote_account: Pubkey,
}

impl NativeStakeAdapter {
    /// Create a native stake adapter pinned to one allowlisted validator vote account.
    #[must_use]
    pub const fn new(
        gateway: Arc<SolanaGateway>,
        signer: Arc<LocalKeypair>,
        vote_account: Pubkey,
    ) -> Self {
        Self {
            gateway,
            signer,
            vote_account,
        }
    }

    /// Derive the create-with-seed stake address used by an enter action.
    ///
    /// # Errors
    ///
    /// Returns [`CookerError`] if Solana rejects the deterministic seed derivation.
    pub fn stake_account_for(&self, action_id: &ActionId) -> Result<Pubkey, CookerError> {
        let seed = stake_seed(action_id);
        Pubkey::create_with_seed(&self.signer.pubkey(), &seed, &STAKE_PROGRAM_ID)
            .map_err(|error| CookerError::Codec(format!("cannot derive stake account: {error}")))
    }

    async fn prepare_enter(
        &self,
        action: &PlannedAction,
        lamports: u64,
        stake_account: Option<&str>,
    ) -> Result<PreparedAction, CookerError> {
        if lamports == 0 || stake_account.is_some() {
            return Err(CookerError::Policy(
                "stake enter requires positive lamports and no existing stake account".to_owned(),
            ));
        }
        let minimum_delegation = self.gateway.stake_minimum_delegation().await?;
        if lamports < minimum_delegation {
            return Err(CookerError::Policy(format!(
                "stake principal {lamports} is below Surfpool minimum delegation {minimum_delegation}"
            )));
        }

        let payer = self.signer.pubkey();
        let stake_address = self.stake_account_for(&action.id)?;
        if self.gateway.account(&stake_address).await?.is_some() {
            return Err(CookerError::Policy(format!(
                "deterministic stake account already exists: {stake_address}"
            )));
        }
        let rent = self
            .gateway
            .minimum_balance_for_rent_exemption(StakeStateV2::size_of())
            .await?;
        if rent > action.max_account_creation_lamports {
            return Err(CookerError::Policy(format!(
                "stake rent {rent} exceeds account-creation ceiling {}",
                action.max_account_creation_lamports
            )));
        }
        let funded_lamports = lamports
            .checked_add(rent)
            .ok_or_else(|| CookerError::Policy("stake funding overflowed".to_owned()))?;
        let required = funded_lamports
            .checked_add(action.max_fee_lamports)
            .ok_or_else(|| CookerError::Policy("stake spend overflowed".to_owned()))?;
        let payer_before = self.gateway.balance(&payer).await?;
        if payer_before < required {
            return Err(CookerError::Policy(format!(
                "native balance {payer_before} cannot cover stake, rent, and fee ceiling {required}"
            )));
        }

        let seed = stake_seed(&action.id);
        let authorized = Authorized::auto(&payer);
        let instructions = stake_instruction::create_account_with_seed_and_delegate_stake(
            &payer,
            &stake_address,
            &payer,
            &seed,
            &self.vote_account,
            &authorized,
            &Lockup::default(),
            funded_lamports,
        );
        let latest = self.gateway.latest_blockhash().await?;
        let signed = build_signed_transaction(&instructions, &self.signer, latest)?;
        Ok(prepared_action(
            action,
            signed,
            vec![
                position_expectation(
                    StakeOperation::Enter,
                    stake_address,
                    payer,
                    self.vote_account,
                    0,
                    lamports,
                    rent,
                    i128::from(funded_lamports),
                ),
                payer_expectation(payer, payer_before, funded_lamports),
            ],
        ))
    }

    async fn prepare_deactivate(
        &self,
        action: &PlannedAction,
        lamports: u64,
        stake_account: Option<&str>,
    ) -> Result<PreparedAction, CookerError> {
        require_zero_lifecycle_lamports(StakeOperation::Deactivate, lamports)?;
        let stake_address = required_stake_address(stake_account)?;
        let position = self.read_position(&stake_address).await?;
        position.require_authorities(self.signer.pubkey())?;
        let delegation = position.delegation()?;
        if delegation.voter_pubkey != self.vote_account {
            return Err(CookerError::Policy(
                "stake position is delegated to a non-allowlisted vote account".to_owned(),
            ));
        }
        if delegation.deactivation_epoch != u64::MAX {
            return Err(CookerError::Policy(
                "stake position is already deactivating or inactive".to_owned(),
            ));
        }
        let payer = self.signer.pubkey();
        let payer_before = self.gateway.balance(&payer).await?;
        if payer_before < action.max_fee_lamports {
            return Err(CookerError::Policy(
                "native balance cannot cover deactivate fee ceiling".to_owned(),
            ));
        }
        let signed = build_signed_transaction(
            &[stake_instruction::deactivate_stake(&stake_address, &payer)],
            &self.signer,
            self.gateway.latest_blockhash().await?,
        )?;
        Ok(prepared_action(
            action,
            signed,
            vec![
                position_expectation(
                    StakeOperation::Deactivate,
                    stake_address,
                    payer,
                    delegation.voter_pubkey,
                    position.account.lamports,
                    delegation.stake,
                    position.rent_reserve(),
                    0,
                ),
                payer_expectation(payer, payer_before, 0),
            ],
        ))
    }

    async fn prepare_withdraw(
        &self,
        action: &PlannedAction,
        lamports: u64,
        stake_account: Option<&str>,
    ) -> Result<PreparedAction, CookerError> {
        require_zero_lifecycle_lamports(StakeOperation::Withdraw, lamports)?;
        let stake_address = required_stake_address(stake_account)?;
        let position = self.read_position(&stake_address).await?;
        position.require_authorities(self.signer.pubkey())?;
        let delegation = position.delegation()?;
        if delegation.voter_pubkey != self.vote_account {
            return Err(CookerError::Policy(
                "stake position is delegated to a non-allowlisted vote account".to_owned(),
            ));
        }
        if delegation.deactivation_epoch == u64::MAX {
            return Err(CookerError::Policy(
                "stake must be deactivated before withdrawal".to_owned(),
            ));
        }
        let payer = self.signer.pubkey();
        let payer_before = self.gateway.balance(&payer).await?;
        if payer_before < action.max_fee_lamports {
            return Err(CookerError::Policy(
                "native balance cannot cover withdraw fee ceiling".to_owned(),
            ));
        }
        let signed = build_signed_transaction(
            &[stake_instruction::withdraw(
                &stake_address,
                &payer,
                &payer,
                position.account.lamports,
                None,
            )],
            &self.signer,
            self.gateway.latest_blockhash().await?,
        )?;
        Ok(prepared_action(
            action,
            signed,
            vec![
                position_expectation(
                    StakeOperation::Withdraw,
                    stake_address,
                    payer,
                    delegation.voter_pubkey,
                    position.account.lamports,
                    delegation.stake,
                    position.rent_reserve(),
                    -i128::from(position.account.lamports),
                ),
                payer_expectation(payer, payer_before, 0),
            ],
        ))
    }

    async fn read_position(&self, address: &Pubkey) -> Result<StakePosition, CookerError> {
        let account = self
            .gateway
            .account(address)
            .await?
            .ok_or_else(|| CookerError::NotFound(format!("native stake account {address}")))?;
        StakePosition::decode(account)
    }

    async fn current_position_snapshot(
        &self,
        address: &Pubkey,
        expected_principal: u64,
    ) -> Result<CurrentPositionSnapshot, CookerError> {
        let Some(account) = self.gateway.account(address).await? else {
            return Ok(CurrentPositionSnapshot::closed());
        };
        let position = StakePosition::decode(account)?;
        let delegation = position.delegation()?;
        let state = if delegation.deactivation_epoch == u64::MAX {
            "active"
        } else {
            "deactivated"
        };
        let semantic_match = position.authorities()
            == Some(Authorized::auto(&self.signer.pubkey()))
            && delegation.voter_pubkey == self.vote_account
            && delegation.stake == expected_principal;
        Ok(CurrentPositionSnapshot {
            state,
            lamports: Some(position.account.lamports),
            activation_epoch: Some(delegation.activation_epoch),
            deactivation_epoch: Some(delegation.deactivation_epoch),
            semantic_match,
        })
    }

    async fn observe_enter(
        &self,
        action: &PlannedAction,
        prepared: &PreparedAction,
        receipt: &mut ChainReceipt,
        record: &TransactionRecord,
        principal: u64,
    ) -> Result<(), CookerError> {
        let payer = self.signer.pubkey();
        let stake_address = self.stake_account_for(&action.id)?;
        let expected = find_expectation(prepared, POSITION_KIND, &stake_address)?;
        let payer_expected = find_expectation(prepared, PAYER_KIND, &payer)?;
        let rent = attribute_u64(expected, "rent_lamports")?;
        let expected_balance = principal
            .checked_add(rent)
            .ok_or_else(|| CookerError::Codec("stake balance overflowed".to_owned()))?;
        let (position_before, position_after) =
            transaction_native_balances(record, &stake_address, "stake enter position")?;
        let position_delta = i128::from(position_after) - i128::from(position_before);
        let current = self
            .current_position_snapshot(&stake_address, principal)
            .await?;
        let current_met = current.state == "closed" || current.semantic_match;
        let position_met = position_before == 0
            && position_after == expected_balance
            && position_delta == i128::from(expected_balance)
            && current_met;
        let (payer_before, payer_after) =
            transaction_native_balances(record, &payer, "stake enter payer")?;
        let expected_debit = expected_balance
            .checked_add(record.fee)
            .ok_or_else(|| CookerError::Codec("stake payer debit overflowed".to_owned()))?;
        let payer_delta = i128::from(payer_after) - i128::from(payer_before);
        let payer_met = payer_delta == -i128::from(expected_debit);
        let fee_met = record.fee <= action.max_fee_lamports;
        receipt.observations = vec![
            position_observation(
                expected,
                position_before,
                position_after,
                &current,
                position_met,
            ),
            payer_observation(payer_expected, payer_before, payer_after, payer_met),
            fee_observation(payer, record.fee, action.max_fee_lamports, fee_met),
        ];
        receipt.postconditions_met = position_met && payer_met && fee_met;
        Ok(())
    }

    async fn observe_deactivate(
        &self,
        action: &PlannedAction,
        prepared: &PreparedAction,
        receipt: &mut ChainReceipt,
        record: &TransactionRecord,
        stake_account: Option<&str>,
    ) -> Result<(), CookerError> {
        let payer = self.signer.pubkey();
        let stake_address = required_stake_address(stake_account)?;
        let expected = find_expectation(prepared, POSITION_KIND, &stake_address)?;
        let payer_expected = find_expectation(prepared, PAYER_KIND, &payer)?;
        let before = attribute_u64(expected, "before_lamports")?;
        let principal = attribute_u64(expected, "principal_lamports")?;
        let (position_before, position_after) =
            transaction_native_balances(record, &stake_address, "stake deactivate position")?;
        let current = self
            .current_position_snapshot(&stake_address, principal)
            .await?;
        let current_met = current.state == "closed" || current.semantic_match;
        let position_met = position_before == before && position_after == before && current_met;
        let (payer_before, payer_after) =
            transaction_native_balances(record, &payer, "stake deactivate payer")?;
        let payer_delta = i128::from(payer_after) - i128::from(payer_before);
        let payer_met = payer_delta == -i128::from(record.fee);
        let fee_met = record.fee <= action.max_fee_lamports;
        receipt.observations = vec![
            position_observation(
                expected,
                position_before,
                position_after,
                &current,
                position_met,
            ),
            payer_observation(payer_expected, payer_before, payer_after, payer_met),
            fee_observation(payer, record.fee, action.max_fee_lamports, fee_met),
        ];
        receipt.postconditions_met = position_met && payer_met && fee_met;
        Ok(())
    }

    async fn observe_withdraw(
        &self,
        action: &PlannedAction,
        prepared: &PreparedAction,
        receipt: &mut ChainReceipt,
        record: &TransactionRecord,
        stake_account: Option<&str>,
    ) -> Result<(), CookerError> {
        let payer = self.signer.pubkey();
        let stake_address = required_stake_address(stake_account)?;
        let expected = find_expectation(prepared, POSITION_KIND, &stake_address)?;
        let payer_expected = find_expectation(prepared, PAYER_KIND, &payer)?;
        let withdrawn = attribute_u64(expected, "before_lamports")?;
        let principal = attribute_u64(expected, "principal_lamports")?;
        let (position_before, position_after) =
            transaction_native_balances(record, &stake_address, "stake withdraw position")?;
        let current = self
            .current_position_snapshot(&stake_address, principal)
            .await?;
        let position_met = position_before == withdrawn && position_after == 0;
        let (payer_before, payer_after) =
            transaction_native_balances(record, &payer, "stake withdraw payer")?;
        let expected_delta = i128::from(withdrawn) - i128::from(record.fee);
        let payer_delta = i128::from(payer_after) - i128::from(payer_before);
        let payer_met = payer_delta == expected_delta;
        let fee_met = record.fee <= action.max_fee_lamports;
        receipt.observations = vec![
            position_observation(
                expected,
                position_before,
                position_after,
                &current,
                position_met,
            ),
            payer_observation(payer_expected, payer_before, payer_after, payer_met),
            fee_observation(payer, record.fee, action.max_fee_lamports, fee_met),
        ];
        receipt.postconditions_met = position_met && payer_met && fee_met;
        Ok(())
    }
}

#[async_trait::async_trait]
impl ActionAdapter for NativeStakeAdapter {
    fn supports(&self, payload: &ActionPayload) -> bool {
        matches!(payload, ActionPayload::StakeLifecycle { .. })
    }

    async fn prepare(
        &self,
        context: &AdapterContext,
        action: &PlannedAction,
    ) -> Result<PreparedAction, CookerError> {
        validate_context(context, &self.gateway, &self.signer)?;
        let ActionPayload::StakeLifecycle {
            operation,
            lamports,
            stake_account,
        } = &action.payload
        else {
            return Err(CookerError::Policy(
                "native stake adapter expected a stake lifecycle action".to_owned(),
            ));
        };
        match operation {
            StakeOperation::Enter => {
                self.prepare_enter(action, *lamports, stake_account.as_deref())
                    .await
            }
            StakeOperation::Deactivate => {
                self.prepare_deactivate(action, *lamports, stake_account.as_deref())
                    .await
            }
            StakeOperation::Withdraw => {
                self.prepare_withdraw(action, *lamports, stake_account.as_deref())
                    .await
            }
        }
    }

    async fn observe(
        &self,
        context: &AdapterContext,
        action: &PlannedAction,
        prepared: &PreparedAction,
    ) -> Result<ChainReceipt, CookerError> {
        validate_context(context, &self.gateway, &self.signer)?;
        validate_prepared(action, prepared, &self.signer)?;
        let ActionPayload::StakeLifecycle {
            operation,
            lamports,
            stake_account,
        } = &action.payload
        else {
            return Err(CookerError::Policy(
                "native stake adapter expected a stake lifecycle action".to_owned(),
            ));
        };
        let mut receipt = observe_until_terminal(&self.gateway, context, prepared).await?;
        if !matches!(
            receipt.status,
            ConfirmationStatus::Confirmed | ConfirmationStatus::Finalized
        ) {
            return Ok(receipt);
        }
        let record = confirmed_record(&self.gateway, prepared).await?;
        match operation {
            StakeOperation::Enter => {
                self.observe_enter(action, prepared, &mut receipt, &record, *lamports)
                    .await?;
            }
            StakeOperation::Deactivate => {
                self.observe_deactivate(
                    action,
                    prepared,
                    &mut receipt,
                    &record,
                    stake_account.as_deref(),
                )
                .await?;
            }
            StakeOperation::Withdraw => {
                self.observe_withdraw(
                    action,
                    prepared,
                    &mut receipt,
                    &record,
                    stake_account.as_deref(),
                )
                .await?;
            }
        }
        if !receipt.postconditions_met {
            receipt.error = Some(format!(
                "native stake {operation:?} postconditions were not proven"
            ));
        }
        Ok(receipt)
    }
}

struct StakePosition {
    account: AccountInfo,
    state: StakeStateV2,
}

struct CurrentPositionSnapshot {
    state: &'static str,
    lamports: Option<u64>,
    activation_epoch: Option<u64>,
    deactivation_epoch: Option<u64>,
    semantic_match: bool,
}

impl CurrentPositionSnapshot {
    const fn closed() -> Self {
        Self {
            state: "closed",
            lamports: None,
            activation_epoch: None,
            deactivation_epoch: None,
            semantic_match: true,
        }
    }
}

impl StakePosition {
    fn decode(account: AccountInfo) -> Result<Self, CookerError> {
        if account.owner != STAKE_PROGRAM_ID {
            return Err(CookerError::Policy(
                "stake account is not owned by the native Stake program".to_owned(),
            ));
        }
        let state = bincode::deserialize(&account.data)
            .map_err(|error| CookerError::Codec(format!("invalid native stake state: {error}")))?;
        Ok(Self { account, state })
    }

    fn authorities(&self) -> Option<Authorized> {
        self.state.authorized()
    }

    fn require_authorities(&self, expected: Pubkey) -> Result<(), CookerError> {
        if self.authorities() != Some(Authorized::auto(&expected)) {
            return Err(CookerError::Policy(
                "stake account staker or withdraw authority is not the local signer".to_owned(),
            ));
        }
        Ok(())
    }

    fn delegation(&self) -> Result<solana_stake_interface::state::Delegation, CookerError> {
        self.state.delegation().ok_or_else(|| {
            CookerError::Policy("stake account is not in delegated state".to_owned())
        })
    }

    fn rent_reserve(&self) -> u64 {
        #[allow(deprecated)]
        self.state.meta().map_or(0, |meta| meta.rent_exempt_reserve)
    }
}

fn stake_seed(action_id: &ActionId) -> String {
    format!("{STAKE_SEED_PREFIX}{}", &action_id.as_str()[..19])
}

fn required_stake_address(value: Option<&str>) -> Result<Pubkey, CookerError> {
    let value = value.ok_or_else(|| {
        CookerError::Policy("stake deactivate/withdraw requires a stake account".to_owned())
    })?;
    Pubkey::from_str(value)
        .map_err(|error| CookerError::Codec(format!("invalid stake account: {error}")))
}

fn require_zero_lifecycle_lamports(
    operation: StakeOperation,
    lamports: u64,
) -> Result<(), CookerError> {
    if lamports != 0 {
        return Err(CookerError::Policy(format!(
            "stake {operation:?} requires zero action lamports"
        )));
    }
    Ok(())
}

fn validate_context(
    context: &AdapterContext,
    gateway: &SolanaGateway,
    signer: &LocalKeypair,
) -> Result<(), CookerError> {
    if &context.rpc_url != gateway.endpoint().as_url() {
        return Err(CookerError::InvalidConfig(
            "adapter context RPC differs from the verified gateway endpoint".to_owned(),
        ));
    }
    let context_signer = Pubkey::from_str(&context.signer).map_err(|error| {
        CookerError::InvalidConfig(format!("adapter context signer is invalid: {error}"))
    })?;
    if context_signer != signer.pubkey() {
        return Err(CookerError::InvalidConfig(
            "adapter context signer differs from the loaded local keypair".to_owned(),
        ));
    }
    Ok(())
}

fn validate_prepared(
    action: &PlannedAction,
    prepared: &PreparedAction,
    signer: &LocalKeypair,
) -> Result<(), CookerError> {
    if prepared.action_id != action.id {
        return Err(CookerError::Codec(
            "prepared stake transaction belongs to another action".to_owned(),
        ));
    }
    let expected_signature = Signature::from_str(&prepared.signature)
        .map_err(|error| CookerError::Codec(format!("invalid prepared signature: {error}")))?;
    let transaction: Transaction = bincode::deserialize(&prepared.transaction)
        .map_err(|error| CookerError::Codec(format!("invalid prepared transaction: {error}")))?;
    if transaction.signatures.first() != Some(&expected_signature)
        || transaction.message.recent_blockhash.to_string() != prepared.recent_blockhash
        || transaction.message.account_keys.first() != Some(&signer.pubkey())
        || !expected_signature.verify(signer.pubkey().as_ref(), &transaction.message_data())
    {
        return Err(CookerError::Codec(
            "prepared stake transaction metadata or signature is inconsistent".to_owned(),
        ));
    }
    Ok(())
}

async fn observe_until_terminal(
    gateway: &SolanaGateway,
    context: &AdapterContext,
    prepared: &PreparedAction,
) -> Result<ChainReceipt, CookerError> {
    let started = Instant::now();
    loop {
        let receipt =
            ChainGateway::observe(gateway, &prepared.action_id, &prepared.signature).await?;
        if matches!(
            receipt.status,
            ConfirmationStatus::Confirmed
                | ConfirmationStatus::Finalized
                | ConfirmationStatus::Failed
        ) {
            return Ok(receipt);
        }
        if receipt.status == ConfirmationStatus::Missing
            && gateway.block_height().await? > prepared.last_valid_block_height
        {
            return Ok(receipt);
        }
        let elapsed = started.elapsed();
        if elapsed >= context.confirmation_timeout {
            return Ok(receipt);
        }
        sleep(OBSERVATION_POLL_INTERVAL.min(context.confirmation_timeout.saturating_sub(elapsed)))
            .await;
    }
}

async fn confirmed_record(
    gateway: &SolanaGateway,
    prepared: &PreparedAction,
) -> Result<TransactionRecord, CookerError> {
    let signature = Signature::from_str(&prepared.signature)
        .map_err(|error| CookerError::Codec(format!("invalid prepared signature: {error}")))?;
    gateway.transaction(&signature).await?.ok_or_else(|| {
        CookerError::Chain("confirmed stake signature had no transaction metadata".to_owned())
    })
}

fn transaction_native_balances(
    record: &TransactionRecord,
    account: &Pubkey,
    label: &str,
) -> Result<(u64, u64), CookerError> {
    let index = record
        .account_keys
        .iter()
        .position(|candidate| candidate == account)
        .ok_or_else(|| {
            CookerError::Codec(format!("transaction metadata omitted {label} account"))
        })?;
    let before = *record.pre_balances.get(index).ok_or_else(|| {
        CookerError::Codec(format!("transaction metadata omitted {label} pre-balance"))
    })?;
    let after = *record.post_balances.get(index).ok_or_else(|| {
        CookerError::Codec(format!("transaction metadata omitted {label} post-balance"))
    })?;
    Ok((before, after))
}

fn prepared_action(
    action: &PlannedAction,
    signed: SignedWireTransaction,
    expectations: Vec<StateExpectation>,
) -> PreparedAction {
    PreparedAction {
        action_id: action.id.clone(),
        signature: signed.signature.to_string(),
        transaction: signed.bytes,
        recent_blockhash: signed.recent_blockhash,
        last_valid_block_height: signed.last_valid_block_height,
        expectations,
    }
}

#[allow(clippy::too_many_arguments)]
fn position_expectation(
    operation: StakeOperation,
    account: Pubkey,
    authority: Pubkey,
    vote_account: Pubkey,
    before_lamports: u64,
    principal_lamports: u64,
    rent_lamports: u64,
    expected_delta: i128,
) -> StateExpectation {
    StateExpectation {
        kind: POSITION_KIND.to_owned(),
        account: account.to_string(),
        expected_delta: Some(expected_delta),
        attributes: BTreeMap::from([
            (
                "operation".to_owned(),
                format!("{operation:?}").to_lowercase(),
            ),
            ("authority".to_owned(), authority.to_string()),
            ("vote_account".to_owned(), vote_account.to_string()),
            ("before_lamports".to_owned(), before_lamports.to_string()),
            (
                "principal_lamports".to_owned(),
                principal_lamports.to_string(),
            ),
            ("rent_lamports".to_owned(), rent_lamports.to_string()),
        ]),
    }
}

fn payer_expectation(payer: Pubkey, before_lamports: u64, action_debit: u64) -> StateExpectation {
    StateExpectation {
        kind: PAYER_KIND.to_owned(),
        account: payer.to_string(),
        expected_delta: None,
        attributes: BTreeMap::from([
            ("before_lamports".to_owned(), before_lamports.to_string()),
            ("action_debit_lamports".to_owned(), action_debit.to_string()),
        ]),
    }
}

fn find_expectation<'a>(
    prepared: &'a PreparedAction,
    kind: &str,
    account: &Pubkey,
) -> Result<&'a StateExpectation, CookerError> {
    prepared
        .expectations
        .iter()
        .find(|item| item.kind == kind && item.account == account.to_string())
        .ok_or_else(|| CookerError::Codec(format!("prepared action omitted {kind}")))
}

fn attribute_u64(expectation: &StateExpectation, key: &str) -> Result<u64, CookerError> {
    expectation
        .attributes
        .get(key)
        .ok_or_else(|| CookerError::Codec(format!("expectation omitted {key}")))?
        .parse()
        .map_err(|error| CookerError::Codec(format!("invalid expectation {key}: {error}")))
}

fn position_observation(
    expected: &StateExpectation,
    transaction_before: u64,
    transaction_after: u64,
    current: &CurrentPositionSnapshot,
    met: bool,
) -> StateExpectation {
    let mut attributes = expected.attributes.clone();
    attributes.insert(
        "transaction_before_lamports".to_owned(),
        transaction_before.to_string(),
    );
    attributes.insert(
        "transaction_after_lamports".to_owned(),
        transaction_after.to_string(),
    );
    attributes.insert("current_state".to_owned(), current.state.to_owned());
    attributes.insert(
        "current_lamports".to_owned(),
        current
            .lamports
            .map_or_else(|| "missing".to_owned(), |value| value.to_string()),
    );
    attributes.insert(
        "current_activation_epoch".to_owned(),
        current
            .activation_epoch
            .map_or_else(|| "missing".to_owned(), |value| value.to_string()),
    );
    attributes.insert(
        "current_deactivation_epoch".to_owned(),
        current
            .deactivation_epoch
            .map_or_else(|| "missing".to_owned(), |value| value.to_string()),
    );
    attributes.insert(
        "current_semantic_match".to_owned(),
        current.semantic_match.to_string(),
    );
    attributes.insert("met".to_owned(), met.to_string());
    StateExpectation {
        kind: expected.kind.clone(),
        account: expected.account.clone(),
        expected_delta: Some(i128::from(transaction_after) - i128::from(transaction_before)),
        attributes,
    }
}

fn payer_observation(
    expected: &StateExpectation,
    transaction_before: u64,
    transaction_after: u64,
    met: bool,
) -> StateExpectation {
    let mut attributes = expected.attributes.clone();
    attributes.insert(
        "transaction_before_lamports".to_owned(),
        transaction_before.to_string(),
    );
    attributes.insert(
        "transaction_after_lamports".to_owned(),
        transaction_after.to_string(),
    );
    attributes.insert("met".to_owned(), met.to_string());
    StateExpectation {
        kind: expected.kind.clone(),
        account: expected.account.clone(),
        expected_delta: Some(i128::from(transaction_after) - i128::from(transaction_before)),
        attributes,
    }
}

fn fee_observation(
    payer: Pubkey,
    actual_fee: u64,
    maximum_fee: u64,
    met: bool,
) -> StateExpectation {
    StateExpectation {
        kind: FEE_KIND.to_owned(),
        account: payer.to_string(),
        expected_delta: Some(-i128::from(actual_fee)),
        attributes: BTreeMap::from([
            ("actual_fee_lamports".to_owned(), actual_fee.to_string()),
            ("max_fee_lamports".to_owned(), maximum_fee.to_string()),
            ("met".to_owned(), met.to_string()),
        ]),
    }
}

#[cfg(test)]
mod tests {
    use cooker_core::{AgentId, RunId};
    use solana_keypair::Keypair;

    use super::*;

    #[test]
    fn action_seed_is_stable_and_within_solana_limit() {
        let id = ActionId::derive(RunId::new(), AgentId::new(), 7, "stake-test");
        let seed = stake_seed(&id);
        assert_eq!(seed.len(), 32);
        assert_eq!(seed, stake_seed(&id));
    }

    #[test]
    fn derived_account_matches_create_with_seed_contract() -> Result<(), CookerError> {
        let signer = Arc::new(LocalKeypair::from_keypair_for_test(Keypair::new()));
        let id = ActionId::derive(RunId::new(), AgentId::new(), 9, "stake-test");
        let expected =
            Pubkey::create_with_seed(&signer.pubkey(), &stake_seed(&id), &STAKE_PROGRAM_ID)
                .map_err(|error| CookerError::Codec(error.to_string()))?;
        assert_ne!(expected, signer.pubkey());
        Ok(())
    }
}
