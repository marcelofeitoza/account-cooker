//! Live failure and signature-reconciliation acceptance against Surfpool.

use std::{io, path::PathBuf, str::FromStr, sync::Arc, time::Duration};

use chrono::Utc;
use cooker_core::{
    ActionAdapter, ActionId, ActionPayload, AdapterContext, AgentId, ChainGateway,
    ConfirmationStatus, CookerError, PlannedAction, RunId,
};
use cooker_solana::{
    LocalKeypair, NativeTransferAdapter, RpcFailureClass, SurfpoolGateway, SurfpoolRpcUrl,
    build_signed_transaction,
};
use reqwest::{Client, redirect::Policy as RedirectPolicy};
use serde_json::{Value, json};
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signature::Signature;
use solana_signer::Signer;
use solana_system_interface::instruction as system_instruction;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::mpsc,
    task::JoinHandle,
    time::{sleep, timeout},
};

const MAX_REQUEST_BYTES: usize = 2 * 1024 * 1024;
const MAX_FEE_LAMPORTS: u64 = 100_000;

#[tokio::test]
#[ignore = "requires scripts/surfpool-start.sh and the harness-funded local keypair"]
#[allow(
    clippy::too_many_lines,
    reason = "the live fault matrix keeps all non-submission and reconciliation assertions explicit"
)]
async fn stale_insufficient_simulation_and_lost_response_reconcile_on_surfpool()
-> Result<(), Box<dyn std::error::Error>> {
    let (gateway, signer, direct_context) = live_context().await?;
    let payer = signer.pubkey();

    let balance = gateway.balance(&payer).await?;
    let insufficient_action = native_action(Keypair::new().pubkey(), balance);
    let direct_adapter = NativeTransferAdapter::new(Arc::clone(&gateway), Arc::clone(&signer));
    let insufficient_error = direct_adapter
        .prepare(&direct_context, &insufficient_action)
        .await
        .err()
        .ok_or_else(|| io::Error::other("insufficient action unexpectedly prepared"))?;
    assert!(matches!(insufficient_error, CookerError::Policy(_)));

    let simulation_destination = Keypair::new().pubkey();
    let overspend = system_instruction::transfer(&payer, &simulation_destination, balance);
    let failed_simulation_wire =
        build_signed_transaction(&[overspend], &signer, gateway.latest_blockhash().await?)?;
    let failed_simulation = gateway.simulate(&failed_simulation_wire.bytes).await?;
    assert!(!failed_simulation.succeeded);
    assert!(failed_simulation.error.is_some());
    assert_eq!(gateway.balance(&simulation_destination).await?, 0);
    assert_eq!(
        count_local_signature(&gateway, failed_simulation_wire.signature).await?,
        0
    );

    let stale_destination = Keypair::new().pubkey();
    let stale_wire = build_signed_transaction(
        &[system_instruction::transfer(&payer, &stale_destination, 1)],
        &signer,
        gateway.latest_blockhash().await?,
    )?;
    let block_height_before_expiry = gateway.block_height().await?;
    let expiry_clock_steps =
        advance_past_block_height(&gateway, stale_wire.last_valid_block_height).await?;
    let block_height_after_expiry = gateway.block_height().await?;
    assert!(block_height_after_expiry > stale_wire.last_valid_block_height);
    let stale_simulation = gateway.simulate(&stale_wire.bytes).await?;
    assert!(!stale_simulation.succeeded);
    let stale_send_error = gateway
        .send_transaction(&stale_wire.bytes)
        .await
        .err()
        .ok_or_else(|| io::Error::other("expired blockhash unexpectedly submitted"))?;
    assert_eq!(stale_send_error.class(), RpcFailureClass::Deterministic);
    assert_eq!(gateway.balance(&stale_destination).await?, 0);
    assert_eq!(
        count_local_signature(&gateway, stale_wire.signature).await?,
        0
    );

    let (proxy_endpoint, mut dropped_send, proxy_task) =
        spawn_loss_proxy(gateway.endpoint().as_url().as_str()).await?;
    let proxy_gateway = Arc::new(SurfpoolGateway::connect(proxy_endpoint).await?);
    let proxy_context = AdapterContext {
        rpc_url: proxy_gateway.endpoint().as_url().clone(),
        signer: payer.to_string(),
        confirmation_timeout: Duration::from_secs(20),
    };
    let proxy_adapter = NativeTransferAdapter::new(Arc::clone(&proxy_gateway), Arc::clone(&signer));
    let reconciled_destination = Keypair::new().pubkey();
    let transfer_amount = 1_000_000;
    let action = native_action(reconciled_destination, transfer_amount);
    let prepared = proxy_adapter.prepare(&proxy_context, &action).await?;
    let simulation = proxy_gateway.simulate(&prepared.transaction).await?;
    assert!(simulation.succeeded, "simulation failed: {simulation:?}");
    let expected_signature = Signature::from_str(&prepared.signature)?;
    let unknown_outcome = proxy_gateway
        .send_transaction(&prepared.transaction)
        .await
        .err()
        .ok_or_else(|| io::Error::other("loss proxy unexpectedly returned a send response"))?;
    assert_eq!(unknown_outcome.class(), RpcFailureClass::UnknownOutcome);
    let surfpool_accepted = timeout(Duration::from_secs(10), dropped_send.recv())
        .await?
        .ok_or_else(|| io::Error::other("loss proxy omitted accepted signature"))?;
    assert_eq!(surfpool_accepted, expected_signature);

    let receipt = direct_adapter
        .observe(&direct_context, &action, &prepared)
        .await?;
    assert!(matches!(
        receipt.status,
        ConfirmationStatus::Confirmed | ConfirmationStatus::Finalized
    ));
    assert!(receipt.postconditions_met, "receipt: {receipt:?}");
    assert_eq!(
        gateway.balance(&reconciled_destination).await?,
        transfer_amount
    );

    let duplicate_error = gateway
        .send_transaction(&prepared.transaction)
        .await
        .err()
        .ok_or_else(|| io::Error::other("Surfpool unexpectedly reprocessed duplicate signature"))?;
    assert_eq!(duplicate_error.class(), RpcFailureClass::Deterministic);
    sleep(Duration::from_millis(300)).await;
    assert_eq!(
        gateway.balance(&reconciled_destination).await?,
        transfer_amount
    );
    assert_eq!(
        count_local_signature(&gateway, expected_signature).await?,
        1
    );
    let record = gateway
        .transaction(&expected_signature)
        .await?
        .ok_or_else(|| io::Error::other("reconciled transaction metadata is missing"))?;
    assert!(record.error.is_none());
    assert!(record.fee <= MAX_FEE_LAMPORTS);
    proxy_task.abort();

    let evidence = json!({
        "schema_version": 1,
        "scenario": "surfpool_fault_and_same_signature_reconciliation",
        "surfpool_version": gateway.identity().surfnet_version,
        "insufficient_rejected_before_signature": true,
        "simulation_failure_signature": sanitize_signature(&failed_simulation_wire.signature),
        "simulation_failure_error": failed_simulation.error,
        "simulation_failure_local_count": 0,
        "stale_signature": sanitize_signature(&stale_wire.signature),
        "stale_blockhash": stale_wire.recent_blockhash,
        "stale_last_valid_block_height": stale_wire.last_valid_block_height,
        "block_height_before_expiry": block_height_before_expiry,
        "block_height_after_expiry": block_height_after_expiry,
        "expiry_clock_steps": expiry_clock_steps,
        "stale_error_class": "deterministic",
        "stale_local_count": 0,
        "lost_response_error_class": "unknown_outcome",
        "reconciled_signature": sanitize_signature(&expected_signature),
        "duplicate_signature_error_class": "deterministic_already_processed",
        "local_signature_count_after_duplicate": 1,
        "destination_delta_lamports": transfer_amount,
        "fee_lamports": record.fee,
        "postconditions_met": receipt.postconditions_met,
    });
    println!("COOKER_FAULT_EVIDENCE={evidence}");
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

async fn live_context()
-> Result<(Arc<SurfpoolGateway>, Arc<LocalKeypair>, AdapterContext), Box<dyn std::error::Error>> {
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let rpc_url =
        std::env::var("COOKER_RPC_URL").unwrap_or_else(|_| "http://127.0.0.1:8899".to_owned());
    let signer_path = std::env::var_os("COOKER_SIGNER_PATH").map_or_else(
        || project_root.join(".surfpool/keys/funder.json"),
        PathBuf::from,
    );
    let endpoint: SurfpoolRpcUrl = rpc_url.parse()?;
    let gateway = Arc::new(SurfpoolGateway::connect(endpoint).await?);
    let signer = Arc::new(LocalKeypair::load(&project_root, signer_path)?);
    let context = AdapterContext {
        rpc_url: gateway.endpoint().as_url().clone(),
        signer: signer.pubkey().to_string(),
        confirmation_timeout: Duration::from_secs(20),
    };
    Ok((gateway, signer, context))
}

fn native_action(destination: Pubkey, lamports: u64) -> PlannedAction {
    let now = Utc::now();
    let run_id = RunId::new();
    let agent_id = AgentId::new();
    PlannedAction {
        id: ActionId::derive(run_id, agent_id, 0, "surfpool-fault-live-v1"),
        run_id,
        agent_id,
        sequence: 0,
        model_version: "surfpool-fault-live-v1".to_owned(),
        scheduled_at: now,
        payload: ActionPayload::NativeTransfer {
            destination: destination.to_string(),
            lamports,
        },
        max_fee_lamports: MAX_FEE_LAMPORTS,
        max_account_creation_lamports: 0,
        created_at: now,
    }
}

async fn count_local_signature(
    gateway: &SurfpoolGateway,
    signature: Signature,
) -> Result<usize, CookerError> {
    Ok(gateway
        .local_signatures(50)
        .await?
        .iter()
        .filter(|record| record.signature == signature)
        .count())
}

async fn advance_past_block_height(
    gateway: &SurfpoolGateway,
    target: u64,
) -> Result<u64, Box<dyn std::error::Error>> {
    let mut submitted = 0_u64;
    while gateway.block_height().await? <= target {
        if submitted >= 200 {
            return Err(
                io::Error::other("Surfpool did not expire blockhash within 200 blocks").into(),
            );
        }
        let current = gateway.epoch_info().await?;
        gateway
            .time_travel_to_slot(
                current
                    .absolute_slot
                    .checked_add(1)
                    .ok_or_else(|| io::Error::other("absolute slot overflowed"))?,
            )
            .await?;
        submitted += 1;
    }
    Ok(submitted)
}

async fn spawn_loss_proxy(
    upstream: &str,
) -> Result<
    (
        SurfpoolRpcUrl,
        mpsc::UnboundedReceiver<Signature>,
        JoinHandle<()>,
    ),
    Box<dyn std::error::Error>,
> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let endpoint: SurfpoolRpcUrl = format!("http://{address}").parse()?;
    let upstream = upstream.to_owned();
    let client = Client::builder()
        .no_proxy()
        .redirect(RedirectPolicy::none())
        .build()?;
    let (sender, receiver) = mpsc::unbounded_channel();
    let task = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let client = client.clone();
            let upstream = upstream.clone();
            let sender = sender.clone();
            tokio::spawn(async move {
                let _ = forward_or_drop(stream, &client, &upstream, &sender).await;
            });
        }
    });
    Ok((endpoint, receiver, task))
}

async fn forward_or_drop(
    mut stream: TcpStream,
    client: &Client,
    upstream: &str,
    dropped_send: &mpsc::UnboundedSender<Signature>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let request = read_http_request(&mut stream).await?;
    let header_end = find_header_end(&request)
        .ok_or_else(|| io::Error::other("proxy request omitted HTTP header terminator"))?;
    let body = &request[header_end..];
    let payload: Value = serde_json::from_slice(body)?;
    let method = payload
        .get("method")
        .and_then(Value::as_str)
        .ok_or_else(|| io::Error::other("proxy request omitted JSON-RPC method"))?;
    let response = client
        .post(upstream)
        .header("content-type", "application/json")
        .body(body.to_vec())
        .send()
        .await?;
    let status = response.status();
    let response_body = response.bytes().await?;
    if method == "sendTransaction" {
        let envelope: Value = serde_json::from_slice(&response_body)?;
        let signature = envelope
            .get("result")
            .and_then(Value::as_str)
            .ok_or_else(|| io::Error::other("Surfpool did not accept proxied transaction"))?;
        dropped_send.send(Signature::from_str(signature)?)?;
        return Ok(());
    }
    let reason = status.canonical_reason().unwrap_or("OK");
    let headers = format!(
        "HTTP/1.1 {} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        status.as_u16(),
        response_body.len()
    );
    stream.write_all(headers.as_bytes()).await?;
    stream.write_all(&response_body).await?;
    stream.shutdown().await?;
    Ok(())
}

async fn read_http_request(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    let mut request = Vec::with_capacity(4096);
    let mut buffer = [0_u8; 4096];
    let mut expected_len = None;
    loop {
        let read = stream.read(&mut buffer).await?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "client closed before completing HTTP request",
            ));
        }
        request.extend_from_slice(&buffer[..read]);
        if request.len() > MAX_REQUEST_BYTES {
            return Err(io::Error::other("proxy request exceeded size limit"));
        }
        if expected_len.is_none()
            && let Some(header_end) = find_header_end(&request)
        {
            let headers = std::str::from_utf8(&request[..header_end])
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            let content_length = headers
                .lines()
                .find_map(|line| {
                    line.split_once(':').and_then(|(name, value)| {
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>())
                    })
                })
                .transpose()
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
                .ok_or_else(|| io::Error::other("proxy request omitted content-length"))?;
            expected_len = Some(header_end + content_length);
        }
        if expected_len.is_some_and(|length| request.len() >= length) {
            return Ok(request);
        }
    }
}

fn find_header_end(request: &[u8]) -> Option<usize> {
    request
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
}
