//! Real-process crash and restart proofs against the pinned local Surfpool harness.

#![forbid(unsafe_code)]

use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    error::Error,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    str::FromStr,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use chrono::{DateTime, TimeDelta, Utc};
use cooker_core::{
    ActionAdapter, ActionId, ActionKind, ActionPayload, ActionState, AdapterContext, AgentId,
    BudgetConfig, ChainGateway, ChainReceipt, Clock, CookerError, PlannedAction, Policy,
    PolicyConfig, RunId, SafetyPolicy, SimulationReceipt, StateStore, VirtualClock,
};
use cooker_runtime::{
    ExecutionCheckpoint, ExecutionResult, FaultInjector, NoFaults, RuntimeEngine, RuntimeSettings,
};
use cooker_solana::{
    LocalKeypair, NativeTransferAdapter, RpcEndpoint, SolanaGateway, TransactionRecord,
};
use cooker_store::{Store, StoreIdentity};
use serde_json::{Value, json};
use solana_pubkey::Pubkey;
use solana_sha256_hasher::hash;
use solana_signature::Signature;
use tempfile::TempDir;
use uuid::Uuid;

mod common;

use common::{advance_past_block_height, sanitize_signature, transaction_native_balances};

#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;

type TestResult = Result<(), Box<dyn Error>>;

const CHILD_MODE: &str = "COOKER_PROCESS_RECOVERY_MODE";
const CASE_INDEX: &str = "COOKER_PROCESS_RECOVERY_CASE_INDEX";
const CHECKPOINT: &str = "COOKER_PROCESS_RECOVERY_CHECKPOINT";
const DATABASE: &str = "COOKER_PROCESS_RECOVERY_DATABASE";
const DESTINATION: &str = "COOKER_PROCESS_RECOVERY_DESTINATION";
const MARKER: &str = "COOKER_PROCESS_RECOVERY_MARKER";
const RESULT: &str = "COOKER_PROCESS_RECOVERY_RESULT";
const STARTED_AT: &str = "COOKER_PROCESS_RECOVERY_STARTED_AT";
const SUBMIT_LOG: &str = "COOKER_PROCESS_RECOVERY_SUBMIT_LOG";
const SURFNET_ID: &str = "COOKER_SURFNET_ID";
const MODEL_VERSION: &str = "surfpool-process-recovery-v1";
const TRANSFER_LAMPORTS: u64 = 1_000_000;
const MAX_FEE_LAMPORTS: u64 = 100_000;
const CHILD_START_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_EXPIRY_SLOT_ADVANCE: u64 = 200;

#[derive(Debug)]
struct PauseAt {
    checkpoint: ExecutionCheckpoint,
    marker: PathBuf,
}

#[derive(Debug)]
struct DurableCountingGateway {
    gateway: Arc<SolanaGateway>,
    submit_log: PathBuf,
}

#[async_trait::async_trait]
impl ChainGateway for DurableCountingGateway {
    async fn simulate(&self, transaction: &[u8]) -> Result<SimulationReceipt, CookerError> {
        ChainGateway::simulate(self.gateway.as_ref(), transaction).await
    }

    async fn submit(&self, transaction: &[u8]) -> Result<String, CookerError> {
        let mut log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.submit_log)
            .map_err(|error| CookerError::Store(error.to_string()))?;
        writeln!(log, "submit")
            .and_then(|()| log.sync_all())
            .map_err(|error| CookerError::Store(error.to_string()))?;
        ChainGateway::submit(self.gateway.as_ref(), transaction).await
    }

    async fn observe(
        &self,
        action_id: &ActionId,
        signature: &str,
    ) -> Result<ChainReceipt, CookerError> {
        ChainGateway::observe(self.gateway.as_ref(), action_id, signature).await
    }

    async fn native_balance(&self, address: &str) -> Result<u64, CookerError> {
        ChainGateway::native_balance(self.gateway.as_ref(), address).await
    }

    async fn block_height(&self) -> Result<u64, CookerError> {
        ChainGateway::block_height(self.gateway.as_ref()).await
    }
}

#[derive(Debug)]
struct RecoveryChildCase<'a> {
    index: usize,
    checkpoint: ExecutionCheckpoint,
    database: &'a Path,
    destination: Pubkey,
    marker: &'a Path,
    result: &'a Path,
    submit_log: &'a Path,
    started_at: DateTime<Utc>,
}

impl FaultInjector for PauseAt {
    fn check(&self, checkpoint: ExecutionCheckpoint) -> Result<(), cooker_core::CookerError> {
        if checkpoint != self.checkpoint {
            return Ok(());
        }

        let marker_temp = self.marker.with_extension("tmp");
        let mut marker = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&marker_temp)
            .map_err(|error| cooker_core::CookerError::Store(error.to_string()))?;
        writeln!(marker, "{}", checkpoint_name(checkpoint))
            .and_then(|()| marker.sync_all())
            .map_err(|error| cooker_core::CookerError::Store(error.to_string()))?;
        fs::rename(&marker_temp, &self.marker)
            .map_err(|error| cooker_core::CookerError::Store(error.to_string()))?;
        loop {
            thread::park();
        }
    }
}

#[tokio::test]
#[ignore = "requires scripts/surfpool-start.sh and the harness-funded local keypair"]
#[allow(clippy::too_many_lines)]
async fn every_process_crash_checkpoint_recovers_on_real_surfpool() -> TestResult {
    let directory = TempDir::new()?;
    let (gateway, signer) = live_resources().await?;
    let source = signer.pubkey();
    let surfnet_id = required_env(SURFNET_ID)?;
    let checkpoints = [
        ExecutionCheckpoint::AfterIntentPersistence,
        ExecutionCheckpoint::AfterPreparedPersistence,
        ExecutionCheckpoint::AfterSimulation,
        ExecutionCheckpoint::AfterSignaturePersistence,
        ExecutionCheckpoint::AfterSendResponseLost,
        ExecutionCheckpoint::AfterConfirmationBeforePromotion,
    ];
    let mut evidence = Vec::with_capacity(checkpoints.len());

    for (index, checkpoint) in checkpoints.into_iter().enumerate() {
        let case_started_at = Utc::now();
        let recovery_at = case_started_at + TimeDelta::seconds(20);
        let database = directory.path().join(format!("case-{index}.sqlite"));
        let marker = directory.path().join(format!("case-{index}.checkpoint"));
        let result_path = directory.path().join(format!("case-{index}.result.json"));
        let submit_log = directory.path().join(format!("case-{index}.submits"));
        let destination = Pubkey::new_unique();
        let action_id = recovery_action_id(index);
        let source_before = gateway.balance(&source).await?;
        let destination_before = gateway.balance(&destination).await?;
        let signatures_before = gateway.local_signatures(10_000).await?;

        let child_case = RecoveryChildCase {
            index,
            checkpoint,
            database: &database,
            destination,
            marker: &marker,
            result: &result_path,
            submit_log: &submit_log,
            started_at: case_started_at,
        };

        let mut child = spawn_child("crash", &child_case)?;
        wait_for_checkpoint(&mut child, &marker)?;
        child.kill()?;
        let killed = child.wait_with_output()?;
        assert!(!killed.status.success(), "crash child exited successfully");
        #[cfg(unix)]
        assert_eq!(killed.status.signal(), Some(9));
        assert_eq!(
            fs::read_to_string(&marker)?.trim(),
            checkpoint_name(checkpoint)
        );
        let crash_submit_attempts = count_submit_attempts(&submit_log)?;
        assert_eq!(
            crash_submit_attempts,
            usize::from(matches!(
                checkpoint,
                ExecutionCheckpoint::AfterSendResponseLost
                    | ExecutionCheckpoint::AfterConfirmationBeforePromotion
            )),
            "checkpoint {checkpoint:?} submitted on the wrong side of the crash"
        );

        let identity = StoreIdentity::surfpool(surfnet_id.clone())?;
        let store = Store::open(&database, identity.clone())?;
        store.verify_integrity()?;
        assert_eq!(store.status_snapshot(recovery_at, None)?.total_actions, 1);
        let crash_record = store.recovery_record(&action_id, case_started_at)?;
        let state_before_recovery = crash_record.state;
        let prepared_before_recovery = crash_record.prepared.clone();
        assert_eq!(
            state_before_recovery,
            expected_crash_state(checkpoint),
            "checkpoint {checkpoint:?} persisted the wrong state"
        );
        assert_crash_record(checkpoint, &crash_record);
        assert!(store.traces_for_run(recovery_run_id(index))?.is_empty());

        let signature_before_recovery = prepared_before_recovery
            .as_ref()
            .map(|prepared| Signature::from_str(&prepared.signature))
            .transpose()?;
        let should_have_landed_before_recovery = matches!(
            checkpoint,
            ExecutionCheckpoint::AfterSendResponseLost
                | ExecutionCheckpoint::AfterConfirmationBeforePromotion
        );
        let pre_recovery_transaction = if let Some(signature) = signature_before_recovery {
            if should_have_landed_before_recovery {
                Some(wait_for_transaction(&gateway, &signature).await?)
            } else {
                assert!(gateway.transaction(&signature).await?.is_none());
                None
            }
        } else {
            None
        };
        let signatures_after_crash = gateway.local_signatures(10_000).await?;
        let crash_signature_occurrences = signature_before_recovery.map_or(0, |signature| {
            signatures_after_crash
                .iter()
                .filter(|record| record.signature == signature)
                .count()
        });
        assert_eq!(
            crash_signature_occurrences,
            usize::from(should_have_landed_before_recovery)
        );
        let source_after_crash = gateway.balance(&source).await?;
        let destination_after_crash = gateway.balance(&destination).await?;
        if let Some(record) = &pre_recovery_transaction {
            assert!(record.error.is_none());
            assert!(record.fee <= MAX_FEE_LAMPORTS);
            assert_eq!(
                destination_after_crash - destination_before,
                TRANSFER_LAMPORTS
            );
            assert_eq!(
                source_before - source_after_crash,
                TRANSFER_LAMPORTS + record.fee
            );
        } else {
            assert_eq!(destination_after_crash, destination_before);
            assert_eq!(source_after_crash, source_before);
        }

        let expiry_slots_advanced = if checkpoint == ExecutionCheckpoint::AfterSignaturePersistence
        {
            let prepared = prepared_before_recovery
                .as_ref()
                .ok_or_else(|| io::Error::other("signature checkpoint omitted signed bytes"))?;
            let advanced = advance_past_block_height(
                &gateway,
                prepared.last_valid_block_height,
                MAX_EXPIRY_SLOT_ADVANCE,
            )
            .await?;
            assert!(gateway.block_height().await? > prepared.last_valid_block_height);
            advanced
        } else {
            0
        };
        drop(store);

        let recovered = run_child("recover", &child_case)?;
        assert!(
            recovered.status.success(),
            "recovery child failed for {checkpoint:?}: stdout={} stderr={}",
            String::from_utf8_lossy(&recovered.stdout),
            String::from_utf8_lossy(&recovered.stderr)
        );
        let recovery_result: Value = serde_json::from_slice(&fs::read(&result_path)?)?;

        let store = Store::open(&database, identity)?;
        store.verify_integrity()?;
        let expected_terminal = if checkpoint == ExecutionCheckpoint::AfterSignaturePersistence {
            ActionState::Expired
        } else {
            ActionState::Confirmed
        };
        assert_eq!(store.get_action_state(&action_id)?, expected_terminal);
        let prepared_after_recovery = store
            .get_prepared(&action_id)?
            .ok_or_else(|| io::Error::other("recovery omitted signed bytes"))?;
        if let Some(prepared) = &prepared_before_recovery {
            assert_eq!(
                prepared, &prepared_after_recovery,
                "checkpoint {checkpoint:?} rebuilt signed bytes"
            );
        }
        let events = store.action_events(&action_id)?;
        assert_eq!(count_events(&events, "prepared"), 1);
        assert_eq!(count_events(&events, "simulation_recorded"), 1);
        assert_eq!(count_events(&events, "submission_recorded"), 1);
        assert_eq!(
            events
                .iter()
                .filter(|event| event.to_state == Some(expected_terminal))
                .count(),
            1
        );
        let snapshot = store.status_snapshot(recovery_at + TimeDelta::seconds(1), None)?;
        assert_eq!(snapshot.total_actions, 1);
        assert_eq!(snapshot.unresolved_actions, 0);
        assert_eq!(store.traces_for_run(recovery_run_id(index))?.len(), 1);
        let submit_attempts = count_submit_attempts(&submit_log)?;
        assert_eq!(
            submit_attempts,
            usize::from(checkpoint != ExecutionCheckpoint::AfterSignaturePersistence)
        );
        let expected_recovery_submits = usize::from(matches!(
            checkpoint,
            ExecutionCheckpoint::AfterIntentPersistence
                | ExecutionCheckpoint::AfterPreparedPersistence
                | ExecutionCheckpoint::AfterSimulation
        ));
        let recovery_submit_attempts = recovery_result["submit_attempts"]
            .as_u64()
            .ok_or_else(|| io::Error::other("recovery child omitted submit attempt count"))?;
        assert_eq!(
            recovery_submit_attempts,
            u64::try_from(expected_recovery_submits)?
        );
        assert_eq!(
            recovery_result["terminal_state"].as_str(),
            Some(state_name(expected_terminal))
        );

        let signature = Signature::from_str(&prepared_after_recovery.signature)?;
        let signatures_after = gateway.local_signatures(10_000).await?;
        let occurrences_before = signatures_before
            .iter()
            .filter(|record| record.signature == signature)
            .count();
        let occurrences_after = signatures_after
            .iter()
            .filter(|record| record.signature == signature)
            .count();
        assert_eq!(occurrences_before, 0);
        let should_land = expected_terminal == ActionState::Confirmed;
        assert_eq!(occurrences_after, usize::from(should_land));

        let source_after = gateway.balance(&source).await?;
        let destination_after = gateway.balance(&destination).await?;
        let destination_credit = destination_after
            .checked_sub(destination_before)
            .ok_or_else(|| io::Error::other("recovery destination balance decreased"))?;
        let source_debit = source_before
            .checked_sub(source_after)
            .ok_or_else(|| io::Error::other("recovery source balance increased"))?;
        let transaction_fee = if should_land {
            let record = gateway
                .transaction(&signature)
                .await?
                .ok_or_else(|| io::Error::other("landed recovery transaction is missing"))?;
            assert!(record.error.is_none());
            assert!(record.fee <= MAX_FEE_LAMPORTS);
            let (record_source_before, record_source_after) =
                transaction_native_balances(&record, &source, "recovery source")?;
            let (record_destination_before, record_destination_after) =
                transaction_native_balances(&record, &destination, "recovery destination")?;
            assert_eq!(record_source_before, source_before);
            assert_eq!(record_source_after, source_after);
            assert_eq!(record_destination_before, destination_before);
            assert_eq!(record_destination_after, destination_after);
            assert_eq!(destination_credit, TRANSFER_LAMPORTS);
            assert_eq!(source_debit, TRANSFER_LAMPORTS + record.fee);
            record.fee
        } else {
            assert!(gateway.transaction(&signature).await?.is_none());
            assert_eq!(destination_credit, 0);
            assert_eq!(source_debit, 0);
            0
        };

        evidence.push(json!({
            "checkpoint": checkpoint_name(checkpoint),
            "child_forcibly_terminated": true,
            "termination_signal": 9,
            "state_before_recovery": state_name(state_before_recovery),
            "terminal_state": state_name(expected_terminal),
            "prepared_durable_before_restart": prepared_before_recovery.is_some(),
            "prepared_reused_exactly": prepared_before_recovery
                .as_ref()
                .is_none_or(|before| before == &prepared_after_recovery),
            "prepared_transaction_sha256": sha256_hex(&prepared_after_recovery.transaction),
            "signature": sanitize_signature(&prepared_after_recovery.signature),
            "signature_sha256": sha256_hex(prepared_after_recovery.signature.as_bytes()),
            "local_signature_occurrences": occurrences_after,
            "local_signature_occurrences_before_recovery": crash_signature_occurrences,
            "expiry_slots_advanced": expiry_slots_advanced,
            "submit_attempts": submit_attempts,
            "crash_submit_attempts": crash_submit_attempts,
            "recovery_submit_attempts": recovery_submit_attempts,
            "logical_action_count": snapshot.total_actions,
            "prepared_event_count": count_events(&events, "prepared"),
            "simulation_event_count": count_events(&events, "simulation_recorded"),
            "submission_event_count": count_events(&events, "submission_recorded"),
            "trace_count": 1,
            "destination_credit_lamports": destination_credit,
            "source_debit_lamports": source_debit,
            "transaction_fee_lamports": transaction_fee,
            "recovery_child": recovery_result,
        }));
    }

    let confirmed = evidence
        .iter()
        .filter(|case| case["terminal_state"] == "confirmed")
        .count();
    let expired = evidence
        .iter()
        .filter(|case| case["terminal_state"] == "expired")
        .count();
    let report = json!({
        "schema_version": 1,
        "scenario": "six_checkpoint_real_process_crash_recovery",
        "surfpool_version": gateway.surfnet_version(),
        "checkpoint_count": evidence.len(),
        "confirmed_actions": confirmed,
        "expired_without_submission": expired,
        "duplicate_logical_actions": 0,
        "duplicate_local_signatures": 0,
        "unresolved_actions": 0,
        "cases": evidence,
    });
    println!("COOKER_PROCESS_RECOVERY_EVIDENCE={report}");
    Ok(())
}

#[tokio::test]
#[ignore = "invoked only as a subprocess by the real-process recovery parent"]
#[allow(
    clippy::too_many_lines,
    reason = "the subprocess protocol keeps crash and restart ordering explicit"
)]
async fn surfpool_recovery_child() -> TestResult {
    let Ok(mode) = env::var(CHILD_MODE) else {
        return Ok(());
    };
    let index = required_env(CASE_INDEX)?.parse::<usize>()?;
    let checkpoint = parse_checkpoint(&required_env(CHECKPOINT)?)?;
    let database = PathBuf::from(required_env(DATABASE)?);
    let destination = Pubkey::from_str(&required_env(DESTINATION)?)?;
    let marker = PathBuf::from(required_env(MARKER)?);
    let result = PathBuf::from(required_env(RESULT)?);
    let submit_log = PathBuf::from(required_env(SUBMIT_LOG)?);
    let started_at = DateTime::parse_from_rfc3339(&required_env(STARTED_AT)?)?.with_timezone(&Utc);
    let surfnet_id = required_env(SURFNET_ID)?;
    let identity = StoreIdentity::surfpool(surfnet_id)?;
    let clock = Arc::new(VirtualClock::new(match mode.as_str() {
        "crash" => started_at,
        "recover" => started_at + TimeDelta::seconds(20),
        other => return Err(format!("invalid recovery child mode: {other}").into()),
    }));
    let (gateway, signer) = live_resources().await?;
    let store = Arc::new(Store::open(&database, identity)?);
    let action_id = recovery_action_id(index);

    if mode == "crash" {
        let action = recovery_action(index, destination, clock.now());
        assert!(store.enqueue_action(&action)?);
        let leases = store.claim_due_actions(
            "crash-child",
            clock.now(),
            clock.now() + TimeDelta::seconds(15),
            1,
        )?;
        let faults: Arc<dyn FaultInjector> = Arc::new(PauseAt { checkpoint, marker });
        let runtime = runtime(
            Arc::clone(&store),
            &gateway,
            &signer,
            destination,
            &submit_log,
            Arc::clone(&clock),
            faults,
        )?;
        runtime
            .execute(
                leases
                    .first()
                    .ok_or("crash child did not claim its action")?,
            )
            .await?;
        return Err("crash child passed its selected checkpoint without pausing".into());
    }

    assert_eq!(store.release_expired_leases(clock.now())?, 1);
    let submit_attempts_before = count_submit_attempts(&submit_log)?;
    let runtime = runtime(
        Arc::clone(&store),
        &gateway,
        &signer,
        destination,
        &submit_log,
        Arc::clone(&clock),
        Arc::new(NoFaults),
    )?;
    let state = store.get_action_state(&action_id)?;
    let execution = if matches!(state, ActionState::Planned | ActionState::Simulated) {
        let leases = store.claim_due_actions(
            "recovery-child",
            clock.now(),
            clock.now() + TimeDelta::minutes(1),
            1,
        )?;
        runtime
            .execute(
                leases
                    .first()
                    .ok_or("recovery child did not claim due action")?,
            )
            .await?
    } else {
        let leases = store.claim_reconciliation_candidates(
            "recovery-child",
            clock.now(),
            clock.now() + TimeDelta::minutes(1),
            1,
        )?;
        runtime
            .reconcile(
                leases
                    .first()
                    .ok_or("recovery child did not claim reconciliation action")?,
            )
            .await?
    };
    let (outcome, signature) = execution_name(&execution);
    let submit_attempts = count_submit_attempts(&submit_log)?
        .checked_sub(submit_attempts_before)
        .ok_or_else(|| io::Error::other("submit attempt count moved backwards"))?;
    write_json_file(
        &result,
        &json!({
            "outcome": outcome,
            "signature": signature.map(|value| sanitize_signature(&value)),
            "terminal_state": state_name(store.get_action_state(&action_id)?),
            "submit_attempts": submit_attempts,
        }),
    )?;
    Ok(())
}

fn runtime(
    store: Arc<Store>,
    gateway: &Arc<SolanaGateway>,
    signer: &Arc<LocalKeypair>,
    destination: Pubkey,
    submit_log: &Path,
    clock: Arc<VirtualClock>,
    faults: Arc<dyn FaultInjector>,
) -> Result<RuntimeEngine, cooker_core::CookerError> {
    let policy: Arc<dyn Policy> = Arc::new(recovery_policy(destination)?);
    let adapter: Arc<dyn ActionAdapter> = Arc::new(NativeTransferAdapter::new(
        Arc::clone(gateway),
        Arc::clone(signer),
    ));
    let store: Arc<dyn StateStore> = store;
    let gateway_for_runtime: Arc<dyn ChainGateway> = Arc::new(DurableCountingGateway {
        gateway: Arc::clone(gateway),
        submit_log: submit_log.to_path_buf(),
    });
    let clock: Arc<dyn Clock> = clock;
    RuntimeEngine::new(
        store,
        gateway_for_runtime,
        policy,
        [(ActionKind::NativeTransfer, adapter)],
        RuntimeSettings {
            adapter_context: AdapterContext {
                rpc_url: gateway.endpoint().as_url().clone(),
                signer: signer.pubkey().to_string(),
                confirmation_timeout: Duration::from_secs(10),
            },
            max_concurrency: 1,
        },
        faults,
        clock,
    )
}

fn recovery_policy(destination: Pubkey) -> Result<SafetyPolicy, cooker_core::CookerError> {
    SafetyPolicy::new(PolicyConfig {
        budgets: BudgetConfig {
            daily_lamports: 100_000_000,
            lifetime_lamports: 1_000_000_000,
            reserve_lamports: 100_000_000,
            max_fee_lamports: MAX_FEE_LAMPORTS,
            max_account_creation_lamports: 0,
        },
        allowed_actions: BTreeSet::from([ActionKind::NativeTransfer]),
        max_action_amounts: BTreeMap::from([(ActionKind::NativeTransfer, TRANSFER_LAMPORTS)]),
        allowed_destinations: BTreeSet::from([destination.to_string()]),
        allowed_mints: BTreeSet::new(),
        native_input_mints: BTreeSet::new(),
        max_slippage_bps: 0,
    })
}

fn recovery_action(index: usize, destination: Pubkey, now: DateTime<Utc>) -> PlannedAction {
    let run_id = recovery_run_id(index);
    let agent_id = recovery_agent_id(index);
    PlannedAction {
        id: ActionId::derive(run_id, agent_id, 0, MODEL_VERSION),
        run_id,
        agent_id,
        sequence: 0,
        model_version: MODEL_VERSION.to_owned(),
        scheduled_at: now,
        payload: ActionPayload::NativeTransfer {
            destination: destination.to_string(),
            lamports: TRANSFER_LAMPORTS,
        },
        max_fee_lamports: MAX_FEE_LAMPORTS,
        max_account_creation_lamports: 0,
        created_at: now,
    }
}

fn recovery_action_id(index: usize) -> ActionId {
    ActionId::derive(
        recovery_run_id(index),
        recovery_agent_id(index),
        0,
        MODEL_VERSION,
    )
}

fn recovery_run_id(index: usize) -> RunId {
    RunId(Uuid::from_u128(0x1_000 + index as u128))
}

fn recovery_agent_id(index: usize) -> AgentId {
    AgentId(Uuid::from_u128(0x2_000 + index as u128))
}

fn spawn_child(mode: &str, case: &RecoveryChildCase<'_>) -> Result<Child, io::Error> {
    let mut command = child_command(mode, case)?;
    command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
}

fn run_child(mode: &str, case: &RecoveryChildCase<'_>) -> Result<std::process::Output, io::Error> {
    child_command(mode, case)?.output()
}

fn child_command(mode: &str, case: &RecoveryChildCase<'_>) -> Result<Command, io::Error> {
    let mut command = Command::new(env::current_exe()?);
    command
        .arg("surfpool_recovery_child")
        .arg("--exact")
        .arg("--ignored")
        .arg("--nocapture")
        .env(CHILD_MODE, mode)
        .env(CASE_INDEX, case.index.to_string())
        .env(CHECKPOINT, checkpoint_name(case.checkpoint))
        .env(DATABASE, case.database)
        .env(DESTINATION, case.destination.to_string())
        .env(MARKER, case.marker)
        .env(RESULT, case.result)
        .env(SUBMIT_LOG, case.submit_log)
        .env(STARTED_AT, case.started_at.to_rfc3339());
    Ok(command)
}

fn wait_for_checkpoint(child: &mut Child, marker: &Path) -> Result<(), Box<dyn Error>> {
    let deadline = Instant::now() + CHILD_START_TIMEOUT;
    loop {
        if marker.is_file() {
            return Ok(());
        }
        if let Some(status) = child.try_wait()? {
            return Err(format!("crash child exited before checkpoint marker: {status}").into());
        }
        if Instant::now() >= deadline {
            child.kill()?;
            let _ = child.wait();
            return Err("timed out waiting for crash checkpoint marker".into());
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn assert_crash_record(checkpoint: ExecutionCheckpoint, record: &cooker_store::RecoveryRecord) {
    let (has_prepared, simulation_count, has_submission, receipt_count) = match checkpoint {
        ExecutionCheckpoint::AfterIntentPersistence => (false, 0, false, 0),
        ExecutionCheckpoint::AfterPreparedPersistence => (true, 0, false, 0),
        ExecutionCheckpoint::AfterSimulation => (true, 1, false, 0),
        ExecutionCheckpoint::AfterSignaturePersistence
        | ExecutionCheckpoint::AfterSendResponseLost => (true, 1, true, 0),
        ExecutionCheckpoint::AfterConfirmationBeforePromotion => (true, 1, true, 1),
    };

    assert_eq!(record.prepared.is_some(), has_prepared);
    assert_eq!(record.simulations.len(), simulation_count);
    assert_eq!(record.submission.is_some(), has_submission);
    assert_eq!(record.receipts.len(), receipt_count);
    assert!(record.active_lease.is_some());
    assert!(!record.events.is_empty());
    assert!(record.events.iter().all(|event| {
        !event
            .to_state
            .is_some_and(cooker_core::ActionState::is_terminal)
    }));
    if let (Some(prepared), Some(submission)) = (&record.prepared, &record.submission) {
        assert_eq!(prepared.signature, submission.signature);
    }
}

async fn wait_for_transaction(
    gateway: &SolanaGateway,
    signature: &Signature,
) -> Result<TransactionRecord, Box<dyn Error>> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(record) = gateway.transaction(signature).await? {
            return Ok(record);
        }
        if Instant::now() >= deadline {
            return Err(io::Error::other(format!(
                "Surfpool did not expose transaction {} before the recovery deadline",
                sanitize_signature(&signature.to_string())
            ))
            .into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn count_submit_attempts(path: &Path) -> Result<usize, io::Error> {
    match fs::read_to_string(path) {
        Ok(contents) => Ok(contents.lines().filter(|line| *line == "submit").count()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error),
    }
}

async fn live_resources() -> Result<(Arc<SolanaGateway>, Arc<LocalKeypair>), Box<dyn Error>> {
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let rpc_url = required_env("COOKER_RPC_URL")?;
    let signer_path = PathBuf::from(required_env("COOKER_SIGNER_PATH")?);
    let endpoint: RpcEndpoint = rpc_url.parse()?;
    let gateway = Arc::new(SolanaGateway::connect(endpoint).await?);
    let signer = Arc::new(LocalKeypair::load(&project_root, signer_path)?);
    Ok((gateway, signer))
}

fn required_env(name: &str) -> Result<String, Box<dyn Error>> {
    env::var(name).map_err(|_| format!("required environment variable is missing: {name}").into())
}

fn expected_crash_state(checkpoint: ExecutionCheckpoint) -> ActionState {
    match checkpoint {
        ExecutionCheckpoint::AfterIntentPersistence
        | ExecutionCheckpoint::AfterPreparedPersistence => ActionState::Planned,
        ExecutionCheckpoint::AfterSimulation => ActionState::Simulated,
        ExecutionCheckpoint::AfterSignaturePersistence
        | ExecutionCheckpoint::AfterSendResponseLost
        | ExecutionCheckpoint::AfterConfirmationBeforePromotion => ActionState::Submitted,
    }
}

fn parse_checkpoint(value: &str) -> Result<ExecutionCheckpoint, Box<dyn Error>> {
    match value {
        "after_intent_persistence" => Ok(ExecutionCheckpoint::AfterIntentPersistence),
        "after_prepared_persistence" => Ok(ExecutionCheckpoint::AfterPreparedPersistence),
        "after_simulation" => Ok(ExecutionCheckpoint::AfterSimulation),
        "after_signature_persistence" => Ok(ExecutionCheckpoint::AfterSignaturePersistence),
        "after_send_response_lost" => Ok(ExecutionCheckpoint::AfterSendResponseLost),
        "after_confirmation_before_promotion" => {
            Ok(ExecutionCheckpoint::AfterConfirmationBeforePromotion)
        }
        other => Err(format!("unknown recovery checkpoint: {other}").into()),
    }
}

const fn checkpoint_name(checkpoint: ExecutionCheckpoint) -> &'static str {
    match checkpoint {
        ExecutionCheckpoint::AfterIntentPersistence => "after_intent_persistence",
        ExecutionCheckpoint::AfterPreparedPersistence => "after_prepared_persistence",
        ExecutionCheckpoint::AfterSimulation => "after_simulation",
        ExecutionCheckpoint::AfterSignaturePersistence => "after_signature_persistence",
        ExecutionCheckpoint::AfterSendResponseLost => "after_send_response_lost",
        ExecutionCheckpoint::AfterConfirmationBeforePromotion => {
            "after_confirmation_before_promotion"
        }
    }
}

const fn state_name(state: ActionState) -> &'static str {
    match state {
        ActionState::Planned => "planned",
        ActionState::Simulated => "simulated",
        ActionState::Submitted => "submitted",
        ActionState::Confirmed => "confirmed",
        ActionState::Rejected => "rejected",
        ActionState::Failed => "failed",
        ActionState::Expired => "expired",
        ActionState::Unknown => "unknown",
        ActionState::Orphaned => "orphaned",
        ActionState::Cancelled => "cancelled",
    }
}

fn execution_name(result: &ExecutionResult) -> (&'static str, Option<&str>) {
    match result {
        ExecutionResult::Confirmed { signature } => ("confirmed", Some(signature)),
        ExecutionResult::Audited { signature } => ("audited", Some(signature)),
        ExecutionResult::Orphaned { signature, .. } => ("orphaned", Some(signature)),
        ExecutionResult::Rejected { .. } => ("rejected", None),
        ExecutionResult::Delayed { .. } => ("delayed", None),
        ExecutionResult::Unknown { signature, .. } => ("unknown", signature.as_deref()),
        ExecutionResult::Expired { signature } => ("expired", Some(signature)),
        ExecutionResult::Failed { .. } => ("failed", None),
    }
}

fn write_json_file(path: &Path, value: &Value) -> Result<(), Box<dyn Error>> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    serde_json::to_writer(&mut file, value)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}

fn count_events(events: &[cooker_store::ActionEventRecord], kind: &str) -> usize {
    events.iter().filter(|event| event.kind == kind).count()
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(hash(bytes).to_bytes())
}
