//! Checks and metadata readers shared by every action adapter.
//!
//! Each adapter used to carry its own copy of these. The copies were identical apart from the
//! noun in their error messages, so the noun is now a parameter and there is one implementation
//! left to audit.

use std::{str::FromStr, time::Duration};

use cooker_core::{
    AdapterContext, ChainGateway, ChainReceipt, ConfirmationStatus, CookerError, PlannedAction,
    PreparedAction, StateExpectation,
};
use solana_pubkey::Pubkey;
use solana_signature::Signature;
use solana_transaction::Transaction;
use tokio::time::{Instant, sleep};

use crate::{LocalKeypair, SignedWireTransaction, SolanaGateway, TransactionRecord};

/// Confirmation polling interval for the loopback harness.
pub(crate) const OBSERVATION_POLL_INTERVAL: Duration = Duration::from_millis(200);
/// Confirmation polling interval for a shared public endpoint.
///
/// A public cluster produces a block roughly every 400 ms and publishes a per-method request
/// ceiling, so a 200 ms poll would spend the whole budget on status reads that cannot yet have
/// changed. Status polling is the single largest consumer of that budget across a fleet, so
/// this is set well above block time.
pub(crate) const PUBLIC_CLUSTER_OBSERVATION_POLL_INTERVAL: Duration = Duration::from_millis(2_500);

/// Reject an adapter context that does not match the verified gateway and loaded signer.
pub(crate) fn validate_context(
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

/// Reject prepared legacy transaction bytes that do not match the action, blockhash, or signer.
///
/// `label` names the adapter in the error text so a failure still says which adapter produced
/// the inconsistent bytes.
pub(crate) fn validate_prepared(
    action: &PlannedAction,
    prepared: &PreparedAction,
    signer: &LocalKeypair,
    label: &str,
) -> Result<(), CookerError> {
    if prepared.action_id != action.id {
        return Err(CookerError::Codec(format!(
            "prepared {label} transaction belongs to another action"
        )));
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
        return Err(CookerError::Codec(format!(
            "prepared {label} transaction metadata or signature is inconsistent"
        )));
    }
    Ok(())
}

/// Poll a submitted signature until it reaches a terminal status, expires, or times out.
pub(crate) async fn observe_until_terminal(
    gateway: &SolanaGateway,
    context: &AdapterContext,
    prepared: &PreparedAction,
) -> Result<ChainReceipt, CookerError> {
    let started = Instant::now();
    let poll_interval = if gateway.endpoint().is_public_cluster() {
        PUBLIC_CLUSTER_OBSERVATION_POLL_INTERVAL
    } else {
        OBSERVATION_POLL_INTERVAL
    };
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
        sleep(poll_interval.min(context.confirmation_timeout.saturating_sub(elapsed))).await;
    }
}

/// Fetch the confirmed transaction metadata a postcondition check reads its deltas from.
pub(crate) async fn confirmed_record(
    gateway: &SolanaGateway,
    prepared: &PreparedAction,
    label: &str,
) -> Result<TransactionRecord, CookerError> {
    let signature = Signature::from_str(&prepared.signature)
        .map_err(|error| CookerError::Codec(format!("invalid prepared signature: {error}")))?;
    gateway.transaction(&signature).await?.ok_or_else(|| {
        CookerError::Chain(format!(
            "confirmed {label} signature had no transaction metadata"
        ))
    })
}

/// Read one account's pre and post native balance out of transaction metadata.
pub(crate) fn transaction_native_balances(
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

/// Assemble the prepared action a signed wire transaction and its expectations describe.
pub(crate) fn prepared_action(
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

/// Find the expectation a prepared action recorded for one account under one kind.
pub(crate) fn find_expectation<'a>(
    prepared: &'a PreparedAction,
    kind: &str,
    account: &Pubkey,
) -> Result<&'a StateExpectation, CookerError> {
    prepared
        .expectations
        .iter()
        .find(|expectation| expectation.kind == kind && expectation.account == account.to_string())
        .ok_or_else(|| {
            CookerError::Codec(format!(
                "prepared transaction omitted {kind} expectation for {account}"
            ))
        })
}

/// Read one `u64` attribute out of a recorded expectation.
pub(crate) fn attribute_u64(expectation: &StateExpectation, key: &str) -> Result<u64, CookerError> {
    expectation
        .attributes
        .get(key)
        .ok_or_else(|| CookerError::Codec(format!("expectation omitted {key}")))?
        .parse()
        .map_err(|error| CookerError::Codec(format!("invalid expectation {key}: {error}")))
}

/// Parse a base58 address, naming the field in the error.
pub(crate) fn parse_pubkey(value: &str, label: &str) -> Result<Pubkey, CookerError> {
    Pubkey::from_str(value).map_err(|error| CookerError::Codec(format!("invalid {label}: {error}")))
}
