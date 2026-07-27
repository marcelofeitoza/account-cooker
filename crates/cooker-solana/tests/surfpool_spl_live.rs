//! Classic SPL Token acceptance against the pinned local Surfpool runtime.

#![forbid(unsafe_code)]

use std::{
    fs, io,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
    time::Duration,
};

use chrono::Utc;
use cooker_core::{
    ActionAdapter, ActionId, ActionPayload, AdapterContext, AgentId, ChainGateway,
    ConfirmationStatus, PlannedAction, RunId,
};
use cooker_solana::{
    LocalKeypair, RpcEndpoint, SolanaGateway, SplTransferAdapter, build_signed_transaction,
    build_signed_transaction_with_signers,
};
use serde_json::json;
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
    instruction::{initialize_mint2, mint_to_checked},
    state::{Account as TokenAccount, Mint},
};
use tokio::time::{Instant, sleep};

const DECIMALS: u8 = 6;
const MINTED_AMOUNT: u64 = 5_000_000;
const TRANSFER_AMOUNT: u64 = 1_234_567;
const MAX_FEE_LAMPORTS: u64 = 100_000;

#[tokio::test]
#[ignore = "requires scripts/surfpool-start.sh and its harness-funded local keypair"]
#[allow(clippy::too_many_lines)]
async fn classic_spl_adapter_proves_creation_transfer_and_exact_overhead()
-> Result<(), Box<dyn std::error::Error>> {
    let started = Instant::now();
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let (gateway, signer, context) = live_context(&project_root).await?;
    let token_program = spl_token_interface::id();
    let mint_keypair = Keypair::new();
    let mint = mint_keypair.pubkey();
    let source = get_associated_token_address(&signer.pubkey(), &mint);
    let destination_owner = Keypair::new().pubkey();
    let destination = get_associated_token_address(&destination_owner, &mint);

    assert!(gateway.account(&mint).await?.is_none());
    assert!(gateway.account(&source).await?.is_none());
    assert!(gateway.account(&destination).await?.is_none());

    let mint_rent = gateway
        .minimum_balance_for_rent_exemption(Mint::LEN)
        .await?;
    let create_mint = system_instruction::create_account(
        &signer.pubkey(),
        &mint,
        mint_rent,
        u64::try_from(Mint::LEN)?,
        &token_program,
    );
    let initialize_mint =
        initialize_mint2(&token_program, &mint, &signer.pubkey(), None, DECIMALS)?;
    let signed_mint = build_signed_transaction_with_signers(
        &[create_mint, initialize_mint],
        &signer,
        &[&mint_keypair],
        gateway.latest_blockhash().await?,
    )?;
    let mint_signature = submit_and_confirm(&gateway, &signed_mint.bytes).await?;
    assert_eq!(mint_signature, signed_mint.signature);

    let create_source = create_associated_token_account_idempotent(
        &signer.pubkey(),
        &signer.pubkey(),
        &mint,
        &token_program,
    );
    let mint_to_source = mint_to_checked(
        &token_program,
        &mint,
        &source,
        &signer.pubkey(),
        &[],
        MINTED_AMOUNT,
        DECIMALS,
    )?;
    let signed_funding = build_signed_transaction(
        &[create_source, mint_to_source],
        &signer,
        gateway.latest_blockhash().await?,
    )?;
    let funding_signature = submit_and_confirm(&gateway, &signed_funding.bytes).await?;
    assert_eq!(funding_signature, signed_funding.signature);

    let mint_account = required_account(&gateway, &mint).await?;
    let mint_state = Mint::unpack(&mint_account.data)?;
    assert_eq!(mint_account.owner, token_program);
    assert!(mint_state.is_initialized);
    assert_eq!(mint_state.decimals, DECIMALS);
    assert_eq!(mint_state.supply, MINTED_AMOUNT);
    let source_before_account = required_account(&gateway, &source).await?;
    let source_before_state = TokenAccount::unpack(&source_before_account.data)?;
    assert_eq!(source_before_account.owner, token_program);
    assert_eq!(source_before_state.mint, mint);
    assert_eq!(source_before_state.owner, signer.pubkey());
    assert_eq!(source_before_state.amount, MINTED_AMOUNT);

    let ata_rent = gateway
        .minimum_balance_for_rent_exemption(TokenAccount::LEN)
        .await?;
    let payer_before = gateway.balance(&signer.pubkey()).await?;
    let now = Utc::now();
    let run_id = RunId::new();
    let agent_id = AgentId::new();
    let action = PlannedAction {
        id: ActionId::derive(run_id, agent_id, 0, "surfpool-classic-spl-live-v1"),
        run_id,
        agent_id,
        sequence: 0,
        model_version: "surfpool-classic-spl-live-v1".to_owned(),
        scheduled_at: now,
        payload: ActionPayload::SplTransfer {
            mint: mint.to_string(),
            destination_owner: destination_owner.to_string(),
            amount: TRANSFER_AMOUNT,
        },
        max_fee_lamports: MAX_FEE_LAMPORTS,
        max_account_creation_lamports: ata_rent,
        created_at: now,
    };
    let adapter = SplTransferAdapter::new(Arc::clone(&gateway), Arc::clone(&signer));
    let prepared = adapter.prepare(&context, &action).await?;

    assert_eq!(prepared.action_id, action.id);
    assert!(prepared.expectations.iter().any(|expectation| {
        expectation.kind == "spl_source_balance_delta"
            && expectation.account == source.to_string()
            && expectation.expected_delta == Some(-i128::from(TRANSFER_AMOUNT))
    }));
    assert!(prepared.expectations.iter().any(|expectation| {
        expectation.kind == "spl_destination_balance_delta"
            && expectation.account == destination.to_string()
            && expectation.expected_delta == Some(i128::from(TRANSFER_AMOUNT))
    }));
    assert!(prepared.expectations.iter().any(|expectation| {
        expectation.kind == "spl_payer_native_overhead"
            && expectation.attributes.get("creation_rent_lamports") == Some(&ata_rent.to_string())
    }));

    let simulation = gateway.simulate(&prepared.transaction).await?;
    assert!(simulation.succeeded, "simulation failed: {simulation:?}");
    let submitted = gateway.submit(&prepared.transaction).await?;
    assert_eq!(submitted, prepared.signature);
    let receipt = adapter.observe(&context, &action, &prepared).await?;
    assert!(matches!(
        receipt.status,
        ConfirmationStatus::Confirmed | ConfirmationStatus::Finalized
    ));
    assert!(receipt.postconditions_met, "receipt: {receipt:?}");
    assert_eq!(receipt.signature, prepared.signature);
    assert!(receipt.observations.iter().all(|observation| {
        observation
            .attributes
            .get("met")
            .is_none_or(|value| value == "true")
    }));

    let source_after_account = required_account(&gateway, &source).await?;
    let destination_after_account = required_account(&gateway, &destination).await?;
    let source_after = TokenAccount::unpack(&source_after_account.data)?;
    let destination_after = TokenAccount::unpack(&destination_after_account.data)?;
    assert_eq!(source_after_account.owner, token_program);
    assert_eq!(destination_after_account.owner, token_program);
    assert_eq!(source_after.mint, mint);
    assert_eq!(destination_after.mint, mint);
    assert_eq!(source_after.owner, signer.pubkey());
    assert_eq!(destination_after.owner, destination_owner);
    assert_eq!(source_after.amount, MINTED_AMOUNT - TRANSFER_AMOUNT);
    assert_eq!(destination_after.amount, TRANSFER_AMOUNT);
    assert_eq!(destination_after_account.lamports, ata_rent);

    let signature = Signature::from_str(&prepared.signature)?;
    let record = gateway
        .transaction(&signature)
        .await?
        .ok_or_else(|| io::Error::other("confirmed SPL transaction metadata is missing"))?;
    assert!(record.error.is_none());
    assert!(record.fee > 0 && record.fee <= MAX_FEE_LAMPORTS);
    assert!(record.account_keys.contains(&mint));
    assert!(record.account_keys.contains(&source));
    assert!(record.account_keys.contains(&destination));
    assert_eq!(record.pre_token_amount(&source, &mint), Some(MINTED_AMOUNT));
    assert_eq!(
        record.post_token_amount(&source, &mint),
        Some(MINTED_AMOUNT - TRANSFER_AMOUNT)
    );
    assert_eq!(
        record.post_token_amount(&destination, &mint),
        Some(TRANSFER_AMOUNT)
    );
    let source_metadata = record
        .post_token_balance(&source, &mint)
        .ok_or_else(|| io::Error::other("source token metadata is missing"))?;
    let destination_metadata = record
        .post_token_balance(&destination, &mint)
        .ok_or_else(|| io::Error::other("destination token metadata is missing"))?;
    assert_eq!(source_metadata.owner, Some(signer.pubkey()));
    assert_eq!(destination_metadata.owner, Some(destination_owner));
    assert_eq!(source_metadata.program_id, Some(token_program));
    assert_eq!(destination_metadata.program_id, Some(token_program));
    assert_eq!(source_metadata.decimals, DECIMALS);
    assert_eq!(destination_metadata.decimals, DECIMALS);

    let payer_after = gateway.balance(&signer.pubkey()).await?;
    let native_debit = payer_before
        .checked_sub(payer_after)
        .ok_or_else(|| io::Error::other("SPL payer balance unexpectedly increased"))?;
    assert_eq!(native_debit, record.fee + ata_rent);
    let overhead = receipt
        .observations
        .iter()
        .find(|observation| observation.kind == "spl_payer_native_overhead")
        .ok_or_else(|| io::Error::other("receipt omitted SPL native overhead proof"))?;
    assert_eq!(
        overhead.attributes.get("actual_creation_rent"),
        Some(&ata_rent.to_string())
    );
    assert_eq!(
        overhead.attributes.get("transaction_fee"),
        Some(&record.fee.to_string())
    );

    let mut later_action = action.clone();
    later_action.id = ActionId::derive(run_id, agent_id, 1, "surfpool-classic-spl-live-v1");
    later_action.sequence = 1;
    later_action.payload = ActionPayload::SplTransfer {
        mint: mint.to_string(),
        destination_owner: destination_owner.to_string(),
        amount: 7,
    };
    later_action.max_account_creation_lamports = 0;
    let later_prepared = adapter.prepare(&context, &later_action).await?;
    assert!(
        gateway
            .simulate(&later_prepared.transaction)
            .await?
            .succeeded
    );
    assert_eq!(
        gateway.submit(&later_prepared.transaction).await?,
        later_prepared.signature
    );
    let later_receipt = adapter
        .observe(&context, &later_action, &later_prepared)
        .await?;
    assert!(
        later_receipt.postconditions_met,
        "receipt: {later_receipt:?}"
    );
    let historical_audit = adapter.observe(&context, &action, &prepared).await?;
    assert!(
        historical_audit.postconditions_met,
        "historical receipt changed after a later transfer: {historical_audit:?}"
    );

    let evidence = json!({
        "schema_version": 1,
        "scenario": "classic_spl_transfer_with_destination_ata_creation",
        "surfpool_version": gateway.surfnet_version(),
        "elapsed_ms": u64::try_from(started.elapsed().as_millis())?,
        "mint": mint.to_string(),
        "source_ata": source.to_string(),
        "destination_owner": destination_owner.to_string(),
        "destination_ata": destination.to_string(),
        "minted_raw": MINTED_AMOUNT,
        "transferred_raw": TRANSFER_AMOUNT,
        "source_before_raw": MINTED_AMOUNT,
        "source_after_raw": source_after.amount,
        "destination_before_raw": 0,
        "destination_after_raw": destination_after.amount,
        "fee_lamports": record.fee,
        "ata_rent_lamports": ata_rent,
        "payer_native_debit_lamports": native_debit,
        "transaction_slot": record.slot,
        "signature": sanitize_signature(&prepared.signature),
        "later_signature": sanitize_signature(&later_prepared.signature),
        "historical_confirmation_reaudits": 1,
        "historical_reaudit_resubmissions": 0,
        "postconditions_met": receipt.postconditions_met
            && later_receipt.postconditions_met
            && historical_audit.postconditions_met,
    });
    if let Some(path) = std::env::var_os("COOKER_SPL_EVIDENCE") {
        write_json(Path::new(&path), &evidence)?;
    }
    println!("COOKER_SPL_EVIDENCE={evidence}");
    Ok(())
}

async fn live_context(
    project_root: &Path,
) -> Result<(Arc<SolanaGateway>, Arc<LocalKeypair>, AdapterContext), Box<dyn std::error::Error>> {
    let rpc_url =
        std::env::var("COOKER_RPC_URL").unwrap_or_else(|_| "http://127.0.0.1:8899".to_owned());
    let signer_path = std::env::var_os("COOKER_SIGNER_PATH").map_or_else(
        || project_root.join(".surfpool/keys/funder.json"),
        PathBuf::from,
    );
    let endpoint: RpcEndpoint = rpc_url.parse()?;
    let gateway = Arc::new(SolanaGateway::connect(endpoint).await?);
    let signer = Arc::new(LocalKeypair::load(project_root, signer_path)?);
    let context = AdapterContext {
        rpc_url: gateway.endpoint().as_url().clone(),
        signer: signer.pubkey().to_string(),
        confirmation_timeout: Duration::from_secs(20),
    };
    Ok((gateway, signer, context))
}

async fn required_account(
    gateway: &SolanaGateway,
    address: &Pubkey,
) -> Result<cooker_solana::AccountInfo, Box<dyn std::error::Error>> {
    gateway
        .account(address)
        .await?
        .ok_or_else(|| io::Error::other(format!("account is missing: {address}")).into())
}

async fn submit_and_confirm(
    gateway: &SolanaGateway,
    transaction: &[u8],
) -> Result<Signature, Box<dyn std::error::Error>> {
    let simulation = gateway.simulate(transaction).await?;
    if !simulation.succeeded {
        return Err(io::Error::other(format!("setup simulation failed: {simulation:?}")).into());
    }
    let signature = Signature::from_str(&gateway.submit(transaction).await?)?;
    let started = Instant::now();
    loop {
        if let Some(record) = gateway.transaction(&signature).await? {
            if let Some(error) = record.error {
                return Err(io::Error::other(format!("setup transaction failed: {error}")).into());
            }
            return Ok(signature);
        }
        if started.elapsed() >= Duration::from_secs(20) {
            return Err(io::Error::other(format!(
                "setup transaction did not confirm: {signature}"
            ))
            .into());
        }
        sleep(Duration::from_millis(100)).await;
    }
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

fn write_json(path: &Path, value: &serde_json::Value) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("json.tmp");
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    fs::write(&temporary, bytes)?;
    fs::rename(temporary, path)?;
    Ok(())
}
