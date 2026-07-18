//! Real Jupiter execution acceptance through the pinned local Surfpool runtime.

use std::{collections::BTreeSet, env, io, path::PathBuf, str::FromStr, sync::Arc, time::Duration};

use chrono::Utc;
use cooker_core::{
    ActionAdapter, ActionId, ActionPayload, AdapterContext, AgentId, ChainGateway,
    ConfirmationStatus, PlannedAction, RunId,
};
use cooker_solana::{
    JupiterAdapter, JupiterApiClient, JupiterInstruction, JupiterPolicy, JupiterQuote,
    JupiterSwapInstructions, LocalKeypair, SurfpoolGateway, SurfpoolRpcUrl,
};
use solana_pubkey::Pubkey;
use spl_associated_token_account_interface::address::get_associated_token_address;

const DEFAULT_INPUT_MINT: &str = "So11111111111111111111111111111111111111112";
const DEFAULT_OUTPUT_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
const DEFAULT_INPUT_AMOUNT: &str = "100000000";
const DEFAULT_DEX_LABEL: &str = "Raydium CLMM";
const DEFAULT_AMMS: &str = "CYbD9RaToYMtWKA7QZyoLahnHdWq553Vm62Lh6qWtuxq";
const DEFAULT_PROGRAMS: &str = "CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK";
const DEFAULT_ACCOUNTS: &str = "D8cy77BBepLMngZx6ZukaTff5hCt1HrWyKk3Hnd9oitf,EdPxg8QaeFSrTYqdWJn6Kezwy9McWncTYueD9eMGCuzR,GviiXg2Xc1xCpyNY36r7h1EAy7uvse5UMkiiyHjRDU6Z,3bWPj5eepJm8CxUzk5MMFMN2CFJkntxKvbmy4zwwtpJd,AA5RaVvyGyZgtmAsJJHT5ZVBxVPtAXuYaMwfgeFJW4Mk,E3HmgD8ru5YZacD7vzrJ9M327dMUrBVpUqCLu9Muw3Er,72jQFwjd14BEhyDfdQsH7D2hS5dN1H6bzsikjkyHyx2D,7k1RJiuyfb3XGq7JzeyYRH6d98gmzqQvmNWyffzk24mF,DN8zwL3mdUndZ1AQG1DkswkDSrCfgmcCNuCSwkv4r93z,F8eHC5cgroHnBUp43L6cRHbAqog2n4h2LSbPKamCTn9Y";
const DEFAULT_WRITABLE_ACCOUNTS: &str = "GviiXg2Xc1xCpyNY36r7h1EAy7uvse5UMkiiyHjRDU6Z,3bWPj5eepJm8CxUzk5MMFMN2CFJkntxKvbmy4zwwtpJd,AA5RaVvyGyZgtmAsJJHT5ZVBxVPtAXuYaMwfgeFJW4Mk,E3HmgD8ru5YZacD7vzrJ9M327dMUrBVpUqCLu9Muw3Er,72jQFwjd14BEhyDfdQsH7D2hS5dN1H6bzsikjkyHyx2D,7k1RJiuyfb3XGq7JzeyYRH6d98gmzqQvmNWyffzk24mF,DN8zwL3mdUndZ1AQG1DkswkDSrCfgmcCNuCSwkv4r93z,F8eHC5cgroHnBUp43L6cRHbAqog2n4h2LSbPKamCTn9Y";
const DEFAULT_LOOKUP_TABLES: &str = "BDqppwFYeMpUicN9xbfoM7FgRnHVW1uTUtrGA7uG2vQg";
const REVIEWED_SIGNER: &str = "CEt8KcTR1MRBnqLqBvtGRMUiaL57cM49E3WyRZDemDDT";
const REVIEWED_QUOTE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../fixtures/surfpool/jupiter/raydium-clmm-sol-usdc-cyb-433411234.quote.json"
));
const REVIEWED_INSTRUCTIONS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../fixtures/surfpool/jupiter/raydium-clmm-sol-usdc-cyb-433411234.instructions.json"
));

#[tokio::test]
#[ignore = "requires a reviewed Jupiter route manifest, funded ATA, and real local Surfpool"]
#[allow(clippy::too_many_lines)] // Acceptance keeps setup, submission, and evidence assertions together.
async fn jupiter_exact_input_swap_accepts_on_real_surfpool()
-> Result<(), Box<dyn std::error::Error>> {
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let rpc_url = env::var("COOKER_RPC_URL").unwrap_or_else(|_| "http://127.0.0.1:8899".to_owned());
    let signer_path = env::var_os("COOKER_SIGNER_PATH").map_or_else(
        || project_root.join(".surfpool/keys/funder.json"),
        PathBuf::from,
    );
    let input_mint =
        env::var("COOKER_JUPITER_INPUT_MINT").unwrap_or_else(|_| DEFAULT_INPUT_MINT.to_owned());
    let output_mint =
        env::var("COOKER_JUPITER_OUTPUT_MINT").unwrap_or_else(|_| DEFAULT_OUTPUT_MINT.to_owned());
    let amount = env::var("COOKER_JUPITER_AMOUNT")
        .unwrap_or_else(|_| DEFAULT_INPUT_AMOUNT.to_owned())
        .parse::<u64>()?;
    let slippage_bps = env::var("COOKER_JUPITER_SLIPPAGE_BPS")
        .unwrap_or_else(|_| "50".to_owned())
        .parse::<u16>()?;
    let max_price_impact_bps = env::var("COOKER_JUPITER_MAX_PRICE_IMPACT_BPS")
        .unwrap_or_else(|_| "50".to_owned())
        .parse::<u16>()?;
    let dex_label =
        env::var("COOKER_JUPITER_DEX_LABEL").unwrap_or_else(|_| DEFAULT_DEX_LABEL.to_owned());

    let input_pubkey = Pubkey::from_str(&input_mint)?;
    let output_pubkey = Pubkey::from_str(&output_mint)?;
    let policy = JupiterPolicy::new(
        BTreeSet::from([input_pubkey, output_pubkey]),
        dex_label,
        pubkey_set("COOKER_JUPITER_ALLOWED_AMMS", DEFAULT_AMMS)?,
        pubkey_set("COOKER_JUPITER_ALLOWED_PROGRAMS", DEFAULT_PROGRAMS)?,
        pubkey_set("COOKER_JUPITER_ALLOWED_ACCOUNTS", DEFAULT_ACCOUNTS)?,
        pubkey_set(
            "COOKER_JUPITER_ALLOWED_LOOKUP_TABLES",
            DEFAULT_LOOKUP_TABLES,
        )?,
        max_price_impact_bps,
    )?
    .with_writable_accounts(pubkey_set(
        "COOKER_JUPITER_ALLOWED_WRITABLE_ACCOUNTS",
        DEFAULT_WRITABLE_ACCOUNTS,
    )?);

    let endpoint: SurfpoolRpcUrl = rpc_url.parse()?;
    let gateway = Arc::new(SurfpoolGateway::connect(endpoint).await?);
    let signer = Arc::new(LocalKeypair::load(&project_root, signer_path)?);
    let api = Arc::new(JupiterApiClient::new(env::var("JUPITER_API_KEY").ok())?);
    let adapter = JupiterAdapter::new(Arc::clone(&gateway), Arc::clone(&signer), api, policy);
    let run_id = RunId::new();
    let agent_id = AgentId::new();
    let action = PlannedAction {
        id: ActionId::derive(run_id, agent_id, 0, "jupiter-surfpool-e2e"),
        run_id,
        agent_id,
        sequence: 0,
        model_version: "jupiter-surfpool-e2e".to_owned(),
        scheduled_at: Utc::now(),
        payload: ActionPayload::JupiterSwap {
            input_mint,
            output_mint,
            amount,
            max_slippage_bps: slippage_bps,
        },
        max_fee_lamports: 100_000,
        max_account_creation_lamports: 5_000_000,
        created_at: Utc::now(),
    };
    let context = AdapterContext {
        rpc_url: gateway.endpoint().as_url().clone(),
        signer: signer.pubkey().to_string(),
        confirmation_timeout: Duration::from_secs(30),
    };

    let plan_source =
        env::var("COOKER_JUPITER_PLAN_SOURCE").unwrap_or_else(|_| "reviewed-fixture".to_owned());
    let mut reviewed_user_account_rebindings = 0;
    let prepared = match plan_source.as_str() {
        "reviewed-fixture" => {
            let quote: JupiterQuote = serde_json::from_str(REVIEWED_QUOTE)?;
            let mut instructions: JupiterSwapInstructions =
                serde_json::from_str(REVIEWED_INSTRUCTIONS)?;
            reviewed_user_account_rebindings = rebind_reviewed_user_accounts(
                &mut instructions,
                input_pubkey,
                output_pubkey,
                signer.pubkey(),
            )?;
            adapter
                .prepare_from_reviewed_plan(&context, &action, &quote, &instructions)
                .await?
        }
        "live" => adapter.prepare(&context, &action).await?,
        other => {
            return Err(io::Error::other(format!(
                "unsupported COOKER_JUPITER_PLAN_SOURCE {other:?}"
            ))
            .into());
        }
    };
    let simulation = gateway.simulate(&prepared.transaction).await?;
    if !simulation.succeeded {
        return Err(io::Error::other(format!(
            "real Surfpool Jupiter simulation failed: {:?}",
            simulation.error
        ))
        .into());
    }
    let submitted = gateway.submit(&prepared.transaction).await?;
    if submitted != prepared.signature {
        return Err(io::Error::other("Surfpool returned a different Jupiter signature").into());
    }
    let receipt = adapter.observe(&context, &action, &prepared).await?;
    if !matches!(
        receipt.status,
        ConfirmationStatus::Confirmed | ConfirmationStatus::Finalized
    ) || !receipt.postconditions_met
    {
        return Err(io::Error::other(format!(
            "Jupiter Surfpool postconditions failed: {receipt:?}"
        ))
        .into());
    }
    if receipt.signature != prepared.signature
        || !receipt
            .observations
            .iter()
            .any(|observation| observation.kind == "jupiter_input_token_delta")
        || !receipt
            .observations
            .iter()
            .any(|observation| observation.kind == "jupiter_output_token_delta")
        || !receipt
            .observations
            .iter()
            .any(|observation| observation.kind == "jupiter_route_metadata")
    {
        return Err(io::Error::other("Jupiter acceptance evidence was incomplete").into());
    }

    let evidence = serde_json::json!({
        "schema_version": 1,
        "adapter": "jupiter_exact_input_swap",
        "surfpool_version": &gateway.identity().surfnet_version,
        "plan_source": plan_source,
        "signature": sanitize_signature(&receipt.signature),
        "slot": receipt.slot,
        "input_amount": amount,
        "max_slippage_bps": slippage_bps,
        "reviewed_user_account_rebindings": reviewed_user_account_rebindings,
        "confirmation_status": receipt.status,
        "postconditions_met": receipt.postconditions_met,
        "observations": receipt.observations,
    });
    println!("COOKER_JUPITER_EVIDENCE={evidence}");
    Ok(())
}

fn rebind_reviewed_user_accounts(
    response: &mut JupiterSwapInstructions,
    input_mint: Pubkey,
    output_mint: Pubkey,
    signer: Pubkey,
) -> Result<usize, Box<dyn std::error::Error>> {
    let reviewed_signer = Pubkey::from_str(REVIEWED_SIGNER)?;
    let replacements = [
        (reviewed_signer, signer),
        (
            get_associated_token_address(&reviewed_signer, &input_mint),
            get_associated_token_address(&signer, &input_mint),
        ),
        (
            get_associated_token_address(&reviewed_signer, &output_mint),
            get_associated_token_address(&signer, &output_mint),
        ),
    ]
    .map(|(old, new)| (old.to_string(), new.to_string()));
    let counts = apply_user_account_replacements(response, &replacements);

    if counts.contains(&0) {
        return Err(io::Error::other(
            "reviewed Jupiter plan omitted a signer or derived user token account",
        )
        .into());
    }
    Ok(counts.into_iter().sum())
}

fn apply_user_account_replacements(
    response: &mut JupiterSwapInstructions,
    replacements: &[(String, String); 3],
) -> [usize; 3] {
    let mut counts = [0_usize; 3];

    for instruction in &mut response.compute_budget_instructions {
        rebind_instruction(instruction, replacements, &mut counts);
    }
    for instruction in &mut response.setup_instructions {
        rebind_instruction(instruction, replacements, &mut counts);
    }
    rebind_instruction(&mut response.swap_instruction, replacements, &mut counts);
    if let Some(instruction) = &mut response.cleanup_instruction {
        rebind_instruction(instruction, replacements, &mut counts);
    }
    if let Some(instruction) = &mut response.token_ledger_instruction {
        rebind_instruction(instruction, replacements, &mut counts);
    }
    for instruction in &mut response.other_instructions {
        rebind_instruction(instruction, replacements, &mut counts);
    }
    counts
}

fn rebind_instruction(
    instruction: &mut JupiterInstruction,
    replacements: &[(String, String); 3],
    counts: &mut [usize; 3],
) {
    for account in &mut instruction.accounts {
        for (index, (old, new)) in replacements.iter().enumerate() {
            if account.pubkey == *old {
                account.pubkey.clone_from(new);
                counts[index] += 1;
                break;
            }
        }
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

#[test]
fn reviewed_plan_rebinding_changes_only_the_three_user_identities()
-> Result<(), Box<dyn std::error::Error>> {
    let mut response: JupiterSwapInstructions = serde_json::from_str(REVIEWED_INSTRUCTIONS)?;
    let before = response.clone();
    let new_signer = Pubkey::new_unique();
    let input_mint = Pubkey::from_str(DEFAULT_INPUT_MINT)?;
    let output_mint = Pubkey::from_str(DEFAULT_OUTPUT_MINT)?;
    let reviewed_signer = Pubkey::from_str(REVIEWED_SIGNER)?;
    let replacements =
        rebind_reviewed_user_accounts(&mut response, input_mint, output_mint, new_signer)?;
    assert!(replacements >= 3);

    let before_text = serde_json::to_string(&before)?;
    let after_text = serde_json::to_string(&response)?;
    assert!(before_text.contains(REVIEWED_SIGNER));
    assert!(!after_text.contains(REVIEWED_SIGNER));
    assert!(after_text.contains(&new_signer.to_string()));
    let reverse = [
        (new_signer, reviewed_signer),
        (
            get_associated_token_address(&new_signer, &input_mint),
            get_associated_token_address(&reviewed_signer, &input_mint),
        ),
        (
            get_associated_token_address(&new_signer, &output_mint),
            get_associated_token_address(&reviewed_signer, &output_mint),
        ),
    ]
    .map(|(old, new)| (old.to_string(), new.to_string()));
    let reverse_counts = apply_user_account_replacements(&mut response, &reverse);
    assert_eq!(reverse_counts.into_iter().sum::<usize>(), replacements);
    assert_eq!(before, response);
    Ok(())
}

fn pubkey_set(
    name: &str,
    reviewed_default: &str,
) -> Result<BTreeSet<Pubkey>, Box<dyn std::error::Error>> {
    let value = env::var(name).unwrap_or_else(|_| reviewed_default.to_owned());
    value
        .split(',')
        .filter(|entry| !entry.trim().is_empty())
        .map(|entry| Ok(Pubkey::from_str(entry.trim())?))
        .collect()
}
