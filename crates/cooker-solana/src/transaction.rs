//! Canonical local construction and signing of legacy and v0 Solana transactions.

use cooker_core::CookerError;
use solana_instruction::Instruction;
use solana_message::{AddressLookupTableAccount, VersionedMessage, v0};
use solana_signature::Signature;
use solana_signer::Signer;
use solana_transaction::{Transaction, versioned::VersionedTransaction};

use crate::{LatestBlockhash, LocalKeypair};

const MAX_WIRE_TRANSACTION_BYTES: usize = 1_232;

/// Signed legacy transaction bytes and the stable signature used for reconciliation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignedWireTransaction {
    /// First transaction signature, deterministically produced before submission.
    pub signature: Signature,
    /// Canonical wire bytes accepted by the Solana JSON-RPC API.
    pub bytes: Vec<u8>,
    /// Blockhash used during signing.
    pub recent_blockhash: String,
    /// Last block height at which the transaction remains valid.
    pub last_valid_block_height: u64,
}

/// Build, sign, verify, and serialize a legacy transaction without exposing secret bytes.
///
/// # Errors
///
/// Returns [`CookerError`] when there are no instructions, signing or serialization fails, the
/// local signature does not verify, or the wire transaction exceeds Solana's packet limit.
pub fn build_signed_transaction(
    instructions: &[Instruction],
    signer: &LocalKeypair,
    latest: LatestBlockhash,
) -> Result<SignedWireTransaction, CookerError> {
    build_signed_transaction_with_signers(instructions, signer, &[], latest)
}

/// Build and sign a legacy transaction with the local payer plus ephemeral account signers.
///
/// Additional signers are intended for newly created local accounts such as mints or stake
/// accounts. The payer remains the validated [`LocalKeypair`], and no secret bytes are serialized
/// outside the signed transaction.
///
/// # Errors
///
/// Returns [`CookerError`] when there are no instructions, signing or serialization fails, the
/// local payer signature does not verify, or the wire transaction exceeds Solana's packet limit.
#[allow(clippy::needless_pass_by_value)] // Consumes a small immutable RPC snapshot at signing.
pub fn build_signed_transaction_with_signers(
    instructions: &[Instruction],
    payer: &LocalKeypair,
    additional_signers: &[&dyn Signer],
    latest: LatestBlockhash,
) -> Result<SignedWireTransaction, CookerError> {
    if instructions.is_empty() {
        return Err(CookerError::Codec(
            "cannot sign a transaction without instructions".to_owned(),
        ));
    }

    let recent_blockhash = latest.blockhash.to_string();
    let mut transaction = Transaction::new_with_payer(instructions, Some(&payer.pubkey()));
    let mut signers: Vec<&dyn Signer> = Vec::with_capacity(additional_signers.len() + 1);
    signers.push(payer.keypair());
    signers.extend_from_slice(additional_signers);
    transaction
        .try_sign(signers.as_slice(), latest.blockhash)
        .map_err(|error| CookerError::Codec(format!("transaction signing failed: {error}")))?;
    let signature = *transaction
        .signatures
        .first()
        .ok_or_else(|| CookerError::Codec("signed transaction had no signature".to_owned()))?;
    if !signature.verify(payer.pubkey().as_ref(), &transaction.message_data()) {
        return Err(CookerError::Codec(
            "transaction signature failed local verification".to_owned(),
        ));
    }

    let bytes = bincode::serialize(&transaction).map_err(|error| {
        CookerError::Codec(format!("transaction serialization failed: {error}"))
    })?;
    if bytes.len() > MAX_WIRE_TRANSACTION_BYTES {
        return Err(CookerError::Codec(format!(
            "transaction is {} bytes; Surfpool accepts at most {MAX_WIRE_TRANSACTION_BYTES}",
            bytes.len()
        )));
    }

    Ok(SignedWireTransaction {
        signature,
        bytes,
        recent_blockhash,
        last_valid_block_height: latest.last_valid_block_height,
    })
}

/// Build, sign, verify, and serialize a v0 transaction using lookup tables read from Surfpool.
///
/// The payer is the only permitted signer. Callers must validate every instruction and lookup
/// table before invoking this function; this boundary only compiles and signs those inputs.
///
/// # Errors
///
/// Returns [`CookerError`] when there are no instructions, message compilation or signing fails,
/// the local signature does not verify, or the wire transaction exceeds Solana's packet limit.
#[allow(clippy::needless_pass_by_value)] // Consumes a small immutable RPC snapshot at signing.
pub fn build_signed_v0_transaction(
    instructions: &[Instruction],
    lookup_tables: &[AddressLookupTableAccount],
    payer: &LocalKeypair,
    latest: LatestBlockhash,
) -> Result<SignedWireTransaction, CookerError> {
    if instructions.is_empty() {
        return Err(CookerError::Codec(
            "cannot sign a transaction without instructions".to_owned(),
        ));
    }

    let recent_blockhash = latest.blockhash.to_string();
    let message = VersionedMessage::V0(
        v0::Message::try_compile(
            &payer.pubkey(),
            instructions,
            lookup_tables,
            latest.blockhash,
        )
        .map_err(|error| CookerError::Codec(format!("v0 message compilation failed: {error}")))?,
    );
    let transaction = VersionedTransaction::try_new(message, &[payer.keypair()])
        .map_err(|error| CookerError::Codec(format!("v0 transaction signing failed: {error}")))?;
    let signature = *transaction
        .signatures
        .first()
        .ok_or_else(|| CookerError::Codec("signed v0 transaction had no signature".to_owned()))?;
    if !signature.verify(payer.pubkey().as_ref(), &transaction.message.serialize()) {
        return Err(CookerError::Codec(
            "v0 transaction signature failed local verification".to_owned(),
        ));
    }

    let bytes = bincode::serialize(&transaction).map_err(|error| {
        CookerError::Codec(format!("v0 transaction serialization failed: {error}"))
    })?;
    if bytes.len() > MAX_WIRE_TRANSACTION_BYTES {
        return Err(CookerError::Codec(format!(
            "v0 transaction is {} bytes; Surfpool accepts at most {MAX_WIRE_TRANSACTION_BYTES}",
            bytes.len()
        )));
    }

    Ok(SignedWireTransaction {
        signature,
        bytes,
        recent_blockhash,
        last_valid_block_height: latest.last_valid_block_height,
    })
}

#[cfg(test)]
mod tests {
    use solana_hash::Hash;
    use solana_instruction::AccountMeta;
    use solana_keypair::Keypair;
    use solana_message::{AddressLookupTableAccount, VersionedMessage};
    use solana_pubkey::Pubkey;
    use solana_system_interface::instruction as system_instruction;
    use spl_associated_token_account_interface::{
        address::get_associated_token_address,
        instruction::create_associated_token_account_idempotent,
    };
    use spl_token_interface::instruction::transfer_checked;

    use super::*;

    #[test]
    fn native_transfer_is_signed_and_round_trips() -> Result<(), Box<dyn std::error::Error>> {
        let local_signer = LocalKeypair::from_keypair_for_test(Keypair::new());
        let latest = LatestBlockhash {
            context_slot: 7,
            blockhash: Hash::new_from_array([9; 32]),
            last_valid_block_height: 42,
        };
        let instruction = system_instruction::transfer(
            &local_signer.pubkey(),
            &Pubkey::new_from_array([3; 32]),
            100,
        );

        let wire_transaction = build_signed_transaction(&[instruction], &local_signer, latest)?;
        let decoded: Transaction = bincode::deserialize(&wire_transaction.bytes)?;

        assert_eq!(
            decoded.signatures.first(),
            Some(&wire_transaction.signature)
        );
        assert_eq!(decoded.message.recent_blockhash, latest.blockhash);
        assert!(
            wire_transaction
                .signature
                .verify(local_signer.pubkey().as_ref(), &decoded.message_data())
        );
        assert_eq!(wire_transaction.last_valid_block_height, 42);

        let gateway_decoded: VersionedTransaction = bincode::deserialize(&wire_transaction.bytes)?;
        assert!(matches!(
            gateway_decoded.message,
            VersionedMessage::Legacy(_)
        ));
        assert_eq!(
            gateway_decoded.signatures.first(),
            Some(&wire_transaction.signature)
        );
        Ok(())
    }

    #[test]
    fn empty_instruction_set_is_rejected() {
        let local_signer = LocalKeypair::from_keypair_for_test(Keypair::new());
        let latest = LatestBlockhash {
            context_slot: 0,
            blockhash: Hash::new_from_array([1; 32]),
            last_valid_block_height: 1,
        };
        assert!(build_signed_transaction(&[], &local_signer, latest).is_err());
    }

    #[test]
    fn checked_spl_transfer_is_signed_and_round_trips() -> Result<(), Box<dyn std::error::Error>> {
        let local_signer = LocalKeypair::from_keypair_for_test(Keypair::new());
        let mint = Pubkey::new_from_array([6; 32]);
        let destination_owner = Pubkey::new_from_array([7; 32]);
        let source = get_associated_token_address(&local_signer.pubkey(), &mint);
        let destination = get_associated_token_address(&destination_owner, &mint);
        let token_program = spl_token_interface::id();
        let instructions = [
            create_associated_token_account_idempotent(
                &local_signer.pubkey(),
                &destination_owner,
                &mint,
                &token_program,
            ),
            transfer_checked(
                &token_program,
                &source,
                &mint,
                &destination,
                &local_signer.pubkey(),
                &[],
                123,
                6,
            )?,
        ];
        let wire_transaction = build_signed_transaction(
            &instructions,
            &local_signer,
            LatestBlockhash {
                context_slot: 8,
                blockhash: Hash::new_from_array([8; 32]),
                last_valid_block_height: 99,
            },
        )?;
        let decoded: Transaction = bincode::deserialize(&wire_transaction.bytes)?;

        assert_eq!(
            decoded.signatures.first(),
            Some(&wire_transaction.signature)
        );
        assert_eq!(decoded.message.instructions.len(), 2);
        assert!(
            wire_transaction
                .signature
                .verify(local_signer.pubkey().as_ref(), &decoded.message_data())
        );
        Ok(())
    }

    #[test]
    fn v0_transaction_is_locally_signed_and_uses_lookup_table()
    -> Result<(), Box<dyn std::error::Error>> {
        let local_signer = LocalKeypair::from_keypair_for_test(Keypair::new());
        let looked_up_account = Pubkey::new_from_array([12; 32]);
        let program = Pubkey::new_from_array([13; 32]);
        let instruction = Instruction {
            program_id: program,
            accounts: vec![AccountMeta::new(looked_up_account, false)],
            data: vec![1, 2, 3],
        };
        let lookup_table = AddressLookupTableAccount {
            key: Pubkey::new_from_array([14; 32]),
            addresses: vec![looked_up_account],
        };
        let latest = LatestBlockhash {
            context_slot: 9,
            blockhash: Hash::new_from_array([15; 32]),
            last_valid_block_height: 123,
        };

        let signed =
            build_signed_v0_transaction(&[instruction], &[lookup_table], &local_signer, latest)?;
        let decoded: VersionedTransaction = bincode::deserialize(&signed.bytes)?;

        assert_eq!(decoded.signatures.first(), Some(&signed.signature));
        assert!(
            signed
                .signature
                .verify(local_signer.pubkey().as_ref(), &decoded.message.serialize())
        );
        let VersionedMessage::V0(message) = decoded.message else {
            return Err("expected a v0 message".into());
        };
        assert_eq!(message.recent_blockhash, latest.blockhash);
        assert_eq!(message.address_table_lookups.len(), 1);
        assert_eq!(message.address_table_lookups[0].writable_indexes, vec![0]);
        Ok(())
    }
}
