//! Live native-stake lifecycle acceptance against the pinned local Surfpool harness.

use std::{
    io,
    path::{Path, PathBuf},
    process::Command,
    str::FromStr,
    sync::Arc,
    time::{Duration, Instant},
};

use chrono::Utc;
use cooker_core::{
    ActionAdapter, ActionId, ActionPayload, AdapterContext, AgentId, ChainGateway, ChainReceipt,
    ConfirmationStatus, PlannedAction, PreparedAction, RunId, StakeOperation,
};
use cooker_solana::{
    LocalKeypair, NativeStakeAdapter, SurfpoolGateway, SurfpoolRpcUrl, TransactionRecord,
};
use serde_json::json;
use solana_pubkey::Pubkey;
use solana_signature::Signature;
use solana_stake_interface::{
    program::ID as STAKE_PROGRAM_ID,
    state::{Authorized, StakeStateV2},
};
use tokio::time::sleep;

const MAX_FEE_LAMPORTS: u64 = 100_000;
const VOTE_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("Vote111111111111111111111111111111111111111");

#[tokio::test]
#[ignore = "requires scripts/surfpool-start.sh and the harness-funded local keypair"]
#[allow(
    clippy::too_many_lines,
    reason = "the live lifecycle keeps every signed state transition and assertion in execution order"
)]
async fn native_stake_full_lifecycle_on_real_surfpool() -> Result<(), Box<dyn std::error::Error>> {
    let started = Instant::now();
    let (gateway, signer, signer_path, context) = live_context().await?;
    let payer = signer.pubkey();
    let listed_vote_accounts = gateway.active_vote_accounts().await?;
    let (vote, vote_setup_signature) =
        create_local_vote_account(gateway.endpoint().as_url().as_str(), &signer_path, payer)?;
    let vote_state = gateway
        .account(&vote)
        .await?
        .ok_or_else(|| io::Error::other("active vote account did not clone into Surfpool"))?;
    assert_eq!(vote_state.owner, VOTE_PROGRAM_ID);

    let minimum_delegation = gateway.stake_minimum_delegation().await?;
    let principal = minimum_delegation.max(1_000_000_000);
    let stake_rent = gateway
        .minimum_balance_for_rent_exemption(StakeStateV2::size_of())
        .await?;
    let adapter = NativeStakeAdapter::new(Arc::clone(&gateway), Arc::clone(&signer), vote);

    let enter = stake_action(0, StakeOperation::Enter, principal, None, stake_rent);
    let stake_account = adapter.stake_account_for(&enter.id)?;
    assert!(gateway.account(&stake_account).await?.is_none());
    let (enter_prepared, enter_receipt, enter_record) =
        execute(&gateway, &adapter, &context, &enter).await?;
    assert_success(&enter_receipt);
    assert!(enter_record.error.is_none());
    let entered_account = gateway
        .account(&stake_account)
        .await?
        .ok_or_else(|| io::Error::other("entered stake account is missing"))?;
    assert_eq!(entered_account.owner, STAKE_PROGRAM_ID);
    assert_eq!(entered_account.lamports, principal + stake_rent);
    let entered_state: StakeStateV2 = bincode::deserialize(&entered_account.data)?;
    assert_eq!(entered_state.authorized(), Some(Authorized::auto(&payer)));
    let entered_delegation = entered_state
        .delegation()
        .ok_or_else(|| io::Error::other("entered stake account was not delegated"))?;
    assert_eq!(entered_delegation.voter_pubkey, vote);
    assert_eq!(entered_delegation.stake, principal);
    assert_eq!(entered_delegation.deactivation_epoch, u64::MAX);

    let deactivate = stake_action(1, StakeOperation::Deactivate, 0, Some(stake_account), 0);
    let (deactivate_prepared, deactivate_receipt, deactivate_record) =
        execute(&gateway, &adapter, &context, &deactivate).await?;
    assert_success(&deactivate_receipt);
    assert!(deactivate_record.error.is_none());
    let deactivated_account = gateway
        .account(&stake_account)
        .await?
        .ok_or_else(|| io::Error::other("deactivated stake account is missing"))?;
    assert_eq!(deactivated_account.lamports, principal + stake_rent);
    let deactivated_state: StakeStateV2 = bincode::deserialize(&deactivated_account.data)?;
    let deactivated_delegation = deactivated_state
        .delegation()
        .ok_or_else(|| io::Error::other("deactivated stake account lost delegation metadata"))?;
    assert_ne!(deactivated_delegation.deactivation_epoch, u64::MAX);

    let before_travel = gateway.epoch_info().await?;
    let target_epoch = before_travel
        .epoch
        .max(deactivated_delegation.deactivation_epoch)
        .checked_add(3)
        .ok_or_else(|| io::Error::other("epoch target overflowed"))?;
    let after_travel = gateway.time_travel_to_epoch(target_epoch).await?;
    assert_eq!(after_travel.epoch, target_epoch);
    assert!(after_travel.absolute_slot > before_travel.absolute_slot);

    let payer_before_withdraw = gateway.balance(&payer).await?;
    let withdraw = stake_action(2, StakeOperation::Withdraw, 0, Some(stake_account), 0);
    let (withdraw_prepared, withdraw_receipt, withdraw_record) =
        execute(&gateway, &adapter, &context, &withdraw).await?;
    assert_success(&withdraw_receipt);
    assert!(withdraw_record.error.is_none());
    assert!(gateway.account(&stake_account).await?.is_none());
    let payer_after_withdraw = gateway.balance(&payer).await?;
    assert_eq!(
        payer_after_withdraw,
        payer_before_withdraw + principal + stake_rent - withdraw_record.fee
    );

    let lifecycle_signatures = [
        Signature::from_str(&enter_prepared.signature)?,
        Signature::from_str(&deactivate_prepared.signature)?,
        Signature::from_str(&withdraw_prepared.signature)?,
    ];
    assert_ne!(lifecycle_signatures[0], lifecycle_signatures[1]);
    assert_ne!(lifecycle_signatures[1], lifecycle_signatures[2]);
    let local_signatures = gateway.local_signatures(50).await?;
    for signature in std::iter::once(vote_setup_signature).chain(lifecycle_signatures) {
        assert_eq!(
            local_signatures
                .iter()
                .filter(|record| record.signature == signature)
                .count(),
            1,
            "lifecycle signature must appear exactly once in Surfpool history"
        );
    }

    let enter_audit = adapter.observe(&context, &enter, &enter_prepared).await?;
    let deactivate_audit = adapter
        .observe(&context, &deactivate, &deactivate_prepared)
        .await?;
    let withdraw_audit = adapter
        .observe(&context, &withdraw, &withdraw_prepared)
        .await?;
    for receipt in [&enter_audit, &deactivate_audit, &withdraw_audit] {
        assert_success(receipt);
        assert!(receipt.observations.iter().any(|observation| {
            observation
                .attributes
                .get("current_state")
                .is_some_and(|state| state == "closed")
        }));
    }

    let evidence = json!({
        "schema_version": 1,
        "scenario": "native_stake_create_delegate_deactivate_epoch_withdraw",
        "surfpool_version": gateway.identity().surfnet_version,
        "elapsed_ms": u64::try_from(started.elapsed().as_millis())?,
        "payer": payer.to_string(),
        "vote_account": vote.to_string(),
        "stake_account": stake_account.to_string(),
        "surfpool_listed_vote_accounts_before_setup": listed_vote_accounts.len(),
        "vote_setup_signature": sanitize_signature(&vote_setup_signature),
        "principal_lamports": principal,
        "rent_lamports": stake_rent,
        "deactivation_epoch": deactivated_delegation.deactivation_epoch,
        "time_travel_from_epoch": before_travel.epoch,
        "time_travel_to_epoch": after_travel.epoch,
        "enter_signature": sanitize_signature(&enter_prepared.signature),
        "deactivate_signature": sanitize_signature(&deactivate_prepared.signature),
        "withdraw_signature": sanitize_signature(&withdraw_prepared.signature),
        "enter_fee_lamports": enter_record.fee,
        "deactivate_fee_lamports": deactivate_record.fee,
        "withdraw_fee_lamports": withdraw_record.fee,
        "post_lifecycle_confirmation_reaudits": 3,
        "post_lifecycle_resubmissions": 0,
        "postconditions_met": enter_receipt.postconditions_met
            && deactivate_receipt.postconditions_met
            && withdraw_receipt.postconditions_met
            && enter_audit.postconditions_met
            && deactivate_audit.postconditions_met
            && withdraw_audit.postconditions_met,
    });
    println!("COOKER_STAKE_EVIDENCE={evidence}");
    Ok(())
}

fn sanitize_signature(signature: &impl ToString) -> String {
    let signature = signature.to_string();
    if signature.len() <= 20 {
        return signature;
    }
    format!(
        "{}...{}",
        &signature[..10],
        &signature[signature.len() - 10..]
    )
}

async fn live_context() -> Result<
    (
        Arc<SurfpoolGateway>,
        Arc<LocalKeypair>,
        PathBuf,
        AdapterContext,
    ),
    Box<dyn std::error::Error>,
> {
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let rpc_url =
        std::env::var("COOKER_RPC_URL").unwrap_or_else(|_| "http://127.0.0.1:8899".to_owned());
    let signer_path = std::env::var_os("COOKER_SIGNER_PATH").map_or_else(
        || project_root.join(".surfpool/keys/funder.json"),
        PathBuf::from,
    );
    let endpoint: SurfpoolRpcUrl = rpc_url.parse()?;
    let gateway = Arc::new(SurfpoolGateway::connect(endpoint).await?);
    let signer = Arc::new(LocalKeypair::load(&project_root, &signer_path)?);
    let context = AdapterContext {
        rpc_url: gateway.endpoint().as_url().clone(),
        signer: signer.pubkey().to_string(),
        confirmation_timeout: Duration::from_secs(20),
    };
    Ok((gateway, signer, signer_path, context))
}

fn create_local_vote_account(
    rpc_url: &str,
    signer_path: &Path,
    payer: Pubkey,
) -> Result<(Pubkey, Signature), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let vote_path = directory.path().join("vote-account.json");
    let identity_path = directory.path().join("validator-identity.json");
    for path in [&vote_path, &identity_path] {
        let output = Command::new("solana-keygen")
            .args(["new", "--silent", "--no-bip39-passphrase", "--outfile"])
            .arg(path)
            .output()?;
        if !output.status.success() {
            return Err(io::Error::other(format!(
                "solana-keygen failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ))
            .into());
        }
    }
    let vote_output = Command::new("solana-keygen")
        .arg("pubkey")
        .arg(&vote_path)
        .output()?;
    if !vote_output.status.success() {
        return Err(io::Error::other("could not derive local vote account address").into());
    }
    let vote = Pubkey::from_str(String::from_utf8(vote_output.stdout)?.trim())?;
    let payer_string = payer.to_string();
    let output = Command::new("solana")
        .args(["--url", rpc_url, "--keypair"])
        .arg(signer_path)
        .args(["--output", "json-compact", "create-vote-account"])
        .arg(&vote_path)
        .arg(&identity_path)
        .arg(&payer_string)
        .args(["--authorized-voter", &payer_string, "--commission", "0"])
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "Surfpool vote-account setup failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ))
        .into());
    }
    let response: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    let signature = response
        .get("signature")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| io::Error::other("vote-account setup omitted its signature"))?;
    Ok((vote, Signature::from_str(signature)?))
}

async fn execute(
    gateway: &SurfpoolGateway,
    adapter: &NativeStakeAdapter,
    context: &AdapterContext,
    action: &PlannedAction,
) -> Result<(PreparedAction, ChainReceipt, TransactionRecord), Box<dyn std::error::Error>> {
    let prepared = adapter.prepare(context, action).await?;
    let simulation = gateway.simulate(&prepared.transaction).await?;
    assert!(simulation.succeeded, "simulation failed: {simulation:?}");
    let submitted = gateway.submit(&prepared.transaction).await?;
    assert_eq!(submitted, prepared.signature);
    let receipt = adapter.observe(context, action, &prepared).await?;
    let signature = Signature::from_str(&prepared.signature)?;
    let deadline = Instant::now() + Duration::from_secs(20);
    let record = loop {
        if let Some(record) = gateway.transaction(&signature).await? {
            break record;
        }
        if Instant::now() >= deadline {
            return Err(io::Error::other("stake transaction metadata timed out").into());
        }
        sleep(Duration::from_millis(100)).await;
    };
    Ok((prepared, receipt, record))
}

fn assert_success(receipt: &ChainReceipt) {
    assert!(matches!(
        receipt.status,
        ConfirmationStatus::Confirmed | ConfirmationStatus::Finalized
    ));
    assert!(receipt.postconditions_met, "receipt: {receipt:?}");
    assert_eq!(receipt.observations.len(), 3);
    assert!(receipt.observations.iter().all(|observation| {
        observation
            .attributes
            .get("met")
            .is_none_or(|value| value == "true")
    }));
}

fn stake_action(
    sequence: u64,
    operation: StakeOperation,
    lamports: u64,
    stake_account: Option<Pubkey>,
    account_creation_cap: u64,
) -> PlannedAction {
    let run_id = RunId::new();
    let agent_id = AgentId::new();
    let now = Utc::now();
    PlannedAction {
        id: ActionId::derive(run_id, agent_id, sequence, "surfpool-native-stake-live-v1"),
        run_id,
        agent_id,
        sequence,
        model_version: "surfpool-native-stake-live-v1".to_owned(),
        scheduled_at: now,
        payload: ActionPayload::StakeLifecycle {
            operation,
            lamports,
            stake_account: stake_account.map(|address| address.to_string()),
        },
        max_fee_lamports: MAX_FEE_LAMPORTS,
        max_account_creation_lamports: account_creation_cap,
        created_at: now,
    }
}
