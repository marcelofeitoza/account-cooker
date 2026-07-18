//! Successful chain acceptance tests run only against the pinned local Surfpool harness.

use std::{io, path::PathBuf, str::FromStr, sync::Arc, time::Duration};

use chrono::Utc;
use cooker_core::{
    ActionAdapter, ActionId, ActionPayload, AdapterContext, AgentId, ChainGateway,
    ConfirmationStatus, CookerError, PlannedAction, RunId,
};
use cooker_solana::{
    LocalKeypair, NativeTransferAdapter, SignedWireTransaction, SplTransferAdapter,
    SurfpoolGateway, SurfpoolRpcUrl, TransactionRecord, build_signed_transaction,
    build_signed_transaction_with_signers,
};
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_program_pack::Pack;
use solana_pubkey::Pubkey;
use solana_signature::Signature;
use solana_signer::Signer;
use solana_system_interface::instruction as system_instruction;
use spl_associated_token_account_interface::{
    address::get_associated_token_address, instruction::create_associated_token_account_idempotent,
};
use spl_token_interface::{
    instruction::{initialize_mint2, mint_to},
    state::{Account as TokenAccount, Mint},
};
use tokio::time::{Instant, sleep};

#[tokio::test]
#[ignore = "requires scripts/surfpool-start.sh and the harness-funded local keypair"]
async fn native_adapter_accepts_on_real_surfpool() -> Result<(), Box<dyn std::error::Error>> {
    let (gateway, local_signer) = live_resources().await?;
    let destination = Keypair::new().pubkey();
    let run_id = RunId::new();
    let agent_id = AgentId::new();
    let action = PlannedAction {
        id: ActionId::derive(run_id, agent_id, 0, "surfpool-e2e"),
        run_id,
        agent_id,
        sequence: 0,
        model_version: "surfpool-e2e".to_owned(),
        scheduled_at: Utc::now(),
        payload: ActionPayload::NativeTransfer {
            destination: destination.to_string(),
            lamports: 1_000_000,
        },
        max_fee_lamports: 100_000,
        max_account_creation_lamports: 0,
        created_at: Utc::now(),
    };
    let context = AdapterContext {
        rpc_url: gateway.endpoint().as_url().clone(),
        signer: local_signer.pubkey().to_string(),
        confirmation_timeout: Duration::from_secs(15),
    };
    let adapter = NativeTransferAdapter::new(Arc::clone(&gateway), Arc::clone(&local_signer));

    let prepared = adapter.prepare(&context, &action).await?;
    let simulation = gateway.simulate(&prepared.transaction).await?;
    assert!(simulation.succeeded, "simulation failed: {simulation:?}");
    let submitted_signature = gateway.submit(&prepared.transaction).await?;
    assert_eq!(submitted_signature, prepared.signature);
    let receipt = adapter.observe(&context, &action, &prepared).await?;

    assert!(matches!(
        receipt.status,
        ConfirmationStatus::Confirmed | ConfirmationStatus::Finalized
    ));
    assert!(receipt.postconditions_met, "receipt: {receipt:?}");
    assert_eq!(receipt.signature, prepared.signature);
    assert_eq!(receipt.observations.len(), 3);
    let signature = Signature::from_str(&receipt.signature)?;
    let record = gateway
        .transaction(&signature)
        .await?
        .ok_or("confirmed native transaction metadata missing")?;
    let (source_before, source_after) =
        transaction_native_balances(&record, &local_signer.pubkey(), "native transfer source")?;
    let (destination_before, destination_after) =
        transaction_native_balances(&record, &destination, "native transfer destination")?;
    let source_debit = source_before
        .checked_sub(source_after)
        .ok_or_else(|| io::Error::other("native transfer source balance increased"))?;
    let destination_credit = destination_after
        .checked_sub(destination_before)
        .ok_or_else(|| io::Error::other("native transfer destination balance decreased"))?;
    assert_eq!(source_debit, 1_000_000 + record.fee);
    assert_eq!(destination_credit, 1_000_000);
    let evidence = serde_json::json!({
        "schema_version": 1,
        "adapter": "native_transfer",
        "surfpool_version": &gateway.identity().surfnet_version,
        "signature": sanitize_signature(&receipt.signature),
        "slot": receipt.slot,
        "source": local_signer.pubkey().to_string(),
        "destination": destination.to_string(),
        "transfer_lamports": 1_000_000,
        "source_before_lamports": source_before,
        "source_after_lamports": source_after,
        "source_debit_lamports": source_debit,
        "destination_before_lamports": destination_before,
        "destination_after_lamports": destination_after,
        "destination_credit_lamports": destination_credit,
        "transaction_fee_lamports": record.fee,
        "confirmation_status": receipt.status,
        "postconditions_met": receipt.postconditions_met,
        "observations": receipt.observations,
    });
    println!("COOKER_NATIVE_EVIDENCE={evidence}");
    Ok(())
}

fn transaction_native_balances(
    record: &TransactionRecord,
    address: &Pubkey,
    label: &str,
) -> Result<(u64, u64), io::Error> {
    let index = record
        .account_keys
        .iter()
        .position(|candidate| candidate == address)
        .ok_or_else(|| io::Error::other(format!("{label} missing from transaction accounts")))?;
    let before = record
        .pre_balances
        .get(index)
        .copied()
        .ok_or_else(|| io::Error::other(format!("{label} pre-balance missing")))?;
    let after = record
        .post_balances
        .get(index)
        .copied()
        .ok_or_else(|| io::Error::other(format!("{label} post-balance missing")))?;
    Ok((before, after))
}

fn sanitize_signature(signature: &str) -> String {
    if signature.len() <= 20 {
        return signature.to_owned();
    }
    format!(
        "{}...{}",
        &signature[..10],
        &signature[signature.len() - 10..]
    )
}

#[tokio::test]
#[ignore = "requires scripts/surfpool-start.sh and the harness-funded local keypair"]
#[allow(clippy::too_many_lines)]
async fn spl_adapter_accepts_created_mint_on_real_surfpool()
-> Result<(), Box<dyn std::error::Error>> {
    const DECIMALS: u8 = 6;
    const INITIAL_SUPPLY: u64 = 25_000_000;
    const TRANSFER_AMOUNT: u64 = 7_500_000;

    let (gateway, local_signer) = live_resources().await?;
    let payer = local_signer.pubkey();
    let mint_keypair = Keypair::new();
    let mint = mint_keypair.pubkey();
    let token_program = spl_token_interface::id();
    let mint_rent = gateway
        .minimum_balance_for_rent_exemption(Mint::LEN)
        .await?;
    let create_mint = [
        system_instruction::create_account(
            &payer,
            &mint,
            mint_rent,
            u64::try_from(Mint::LEN)?,
            &token_program,
        ),
        initialize_mint2(&token_program, &mint, &payer, None, DECIMALS)?,
    ];
    let mint_setup = signed_setup_transaction(
        &gateway,
        &local_signer,
        &create_mint,
        &[&mint_keypair as &dyn Signer],
    )
    .await?;
    assert!(
        submit_confirmed(&gateway, &mint_setup)
            .await?
            .error
            .is_none()
    );

    let source = get_associated_token_address(&payer, &mint);
    let fund_source = [
        create_associated_token_account_idempotent(&payer, &payer, &mint, &token_program),
        mint_to(&token_program, &mint, &source, &payer, &[], INITIAL_SUPPLY)?,
    ];
    let source_setup = signed_setup_transaction(&gateway, &local_signer, &fund_source, &[]).await?;
    let source_setup_record = submit_confirmed(&gateway, &source_setup).await?;
    assert_eq!(
        source_setup_record.post_token_amount(&source, &mint),
        Some(INITIAL_SUPPLY)
    );

    let destination_owner = Keypair::new().pubkey();
    let destination = get_associated_token_address(&destination_owner, &mint);
    assert!(gateway.account(&destination).await?.is_none());
    let action = spl_action(mint, destination_owner, TRANSFER_AMOUNT);
    let context = AdapterContext {
        rpc_url: gateway.endpoint().as_url().clone(),
        signer: payer.to_string(),
        confirmation_timeout: Duration::from_secs(15),
    };
    let adapter = SplTransferAdapter::new(Arc::clone(&gateway), Arc::clone(&local_signer));
    let prepared = adapter.prepare(&context, &action).await?;
    let simulation = gateway.simulate(&prepared.transaction).await?;
    assert!(simulation.succeeded, "simulation failed: {simulation:?}");
    let submitted_signature = gateway.submit(&prepared.transaction).await?;
    assert_eq!(submitted_signature, prepared.signature);
    let receipt = adapter.observe(&context, &action, &prepared).await?;

    assert!(matches!(
        receipt.status,
        ConfirmationStatus::Confirmed | ConfirmationStatus::Finalized
    ));
    assert!(receipt.postconditions_met, "receipt: {receipt:?}");
    assert_eq!(receipt.observations.len(), 5);
    let source_account = gateway
        .account(&source)
        .await?
        .ok_or("source ATA missing")?;
    let destination_account = gateway
        .account(&destination)
        .await?
        .ok_or("destination ATA missing")?;
    assert_eq!(source_account.owner, token_program);
    assert_eq!(destination_account.owner, token_program);
    let source_state = TokenAccount::unpack(&source_account.data)?;
    let destination_state = TokenAccount::unpack(&destination_account.data)?;
    assert_eq!(source_state.mint, mint);
    assert_eq!(source_state.owner, payer);
    assert_eq!(source_state.amount, INITIAL_SUPPLY - TRANSFER_AMOUNT);
    assert_eq!(destination_state.mint, mint);
    assert_eq!(destination_state.owner, destination_owner);
    assert_eq!(destination_state.amount, TRANSFER_AMOUNT);

    let signature = Signature::from_str(&receipt.signature)?;
    let record = gateway
        .transaction(&signature)
        .await?
        .ok_or("confirmed SPL transaction metadata missing")?;
    assert_eq!(
        record.post_token_amount(&source, &mint),
        Some(INITIAL_SUPPLY - TRANSFER_AMOUNT)
    );
    assert_eq!(
        record.post_token_amount(&destination, &mint),
        Some(TRANSFER_AMOUNT)
    );
    eprintln!(
        "spl signature={} mint={} source_delta=-{} destination_delta={} fee={} ata_rent={}",
        receipt.signature,
        mint,
        TRANSFER_AMOUNT,
        TRANSFER_AMOUNT,
        record.fee,
        destination_account.lamports
    );
    Ok(())
}

async fn live_resources()
-> Result<(Arc<SurfpoolGateway>, Arc<LocalKeypair>), Box<dyn std::error::Error>> {
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let rpc_url =
        std::env::var("COOKER_RPC_URL").unwrap_or_else(|_| "http://127.0.0.1:8899".to_owned());
    let signer_path = std::env::var_os("COOKER_SIGNER_PATH").map_or_else(
        || project_root.join(".surfpool/keys/funder.json"),
        PathBuf::from,
    );
    let endpoint: SurfpoolRpcUrl = rpc_url.parse()?;
    let gateway = Arc::new(SurfpoolGateway::connect(endpoint).await?);
    let local_signer = Arc::new(LocalKeypair::load(&project_root, signer_path)?);
    Ok((gateway, local_signer))
}

async fn signed_setup_transaction(
    gateway: &SurfpoolGateway,
    payer: &LocalKeypair,
    instructions: &[Instruction],
    additional_signers: &[&dyn Signer],
) -> Result<SignedWireTransaction, CookerError> {
    let latest = gateway.latest_blockhash().await?;
    let signed = if additional_signers.is_empty() {
        build_signed_transaction(instructions, payer, latest)?
    } else {
        build_signed_transaction_with_signers(instructions, payer, additional_signers, latest)?
    };
    let simulation = gateway.simulate(&signed.bytes).await?;
    if !simulation.succeeded {
        return Err(CookerError::Chain(format!(
            "setup transaction simulation failed: {:?}",
            simulation.error
        )));
    }
    Ok(signed)
}

async fn submit_confirmed(
    gateway: &SurfpoolGateway,
    signed: &SignedWireTransaction,
) -> Result<TransactionRecord, CookerError> {
    let submitted = gateway.submit(&signed.bytes).await?;
    if submitted != signed.signature.to_string() {
        return Err(CookerError::UnknownOutcome(
            "Surfpool setup signature differed from local signature".to_owned(),
        ));
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(record) = gateway.transaction(&signed.signature).await? {
            return Ok(record);
        }
        if Instant::now() >= deadline {
            return Err(CookerError::Chain(
                "setup transaction did not confirm before timeout".to_owned(),
            ));
        }
        sleep(Duration::from_millis(100)).await;
    }
}

fn spl_action(mint: Pubkey, destination_owner: Pubkey, amount: u64) -> PlannedAction {
    let run_id = RunId::new();
    let agent_id = AgentId::new();
    PlannedAction {
        id: ActionId::derive(run_id, agent_id, 0, "surfpool-spl-e2e"),
        run_id,
        agent_id,
        sequence: 0,
        model_version: "surfpool-spl-e2e".to_owned(),
        scheduled_at: Utc::now(),
        payload: ActionPayload::SplTransfer {
            mint: mint.to_string(),
            destination_owner: destination_owner.to_string(),
            amount,
        },
        max_fee_lamports: 100_000,
        max_account_creation_lamports: 3_000_000,
        created_at: Utc::now(),
    }
}
