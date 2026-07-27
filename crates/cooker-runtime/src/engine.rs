//! One-action execution state machine and reconciliation path.

use std::{collections::BTreeMap, fmt, sync::Arc};

use cooker_core::{
    ActionAdapter, ActionKind, ActionLease, ActionPayload, ActionState, AdapterContext,
    ChainGateway, ChainReceipt, Clock, ConfirmationStatus, CookerError, PlannedAction, Policy,
    PolicyDecision, StateStore, TraceEvent, TraceOutcome,
};

use crate::{ExecutionCheckpoint, FaultInjector};

/// Observable result of one claimed logical action.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExecutionResult {
    /// Adapter postconditions were confirmed.
    Confirmed {
        /// Stable local signature.
        signature: String,
    },
    /// A prior confirmation and its postconditions were re-verified.
    Audited {
        /// Stable local signature.
        signature: String,
    },
    /// A prior confirmation was downgraded or lost its postconditions.
    Orphaned {
        /// Stable local signature.
        signature: String,
        /// Stable correction reason.
        reason: String,
    },
    /// Policy permanently rejected the action.
    Rejected {
        /// Stable policy reason.
        reason: String,
    },
    /// Policy requested a delay; no transaction was built.
    Delayed {
        /// Stable policy reason.
        reason: String,
    },
    /// Submission may have landed and requires reconciliation.
    Unknown {
        /// Stable local signature when already prepared.
        signature: Option<String>,
        /// Sanitized failure reason.
        reason: String,
    },
    /// A missing transaction expired and cannot land.
    Expired {
        /// Stable local signature.
        signature: String,
    },
    /// The network confirmed transaction failure.
    Failed {
        /// Sanitized chain failure.
        reason: String,
    },
}

/// Non-service runtime settings grouped for explicit construction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeSettings {
    /// Verified local adapter context.
    pub adapter_context: AdapterContext,
    /// Maximum simultaneous action workers.
    pub max_concurrency: usize,
}

/// Runtime dependencies shared by a bounded worker pool.
pub struct RuntimeEngine {
    pub(crate) store: Arc<dyn StateStore>,
    gateway: Arc<dyn ChainGateway>,
    policy: Arc<dyn Policy>,
    adapters: BTreeMap<ActionKind, Arc<dyn ActionAdapter>>,
    context: AdapterContext,
    faults: Arc<dyn FaultInjector>,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) max_concurrency: usize,
}

impl fmt::Debug for RuntimeEngine {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeEngine")
            .field("adapters", &self.adapters.keys().collect::<Vec<_>>())
            .field("context", &self.context)
            .field("max_concurrency", &self.max_concurrency)
            .finish_non_exhaustive()
    }
}

impl RuntimeEngine {
    /// Base58 address of the only signer accepted by this engine's adapter context.
    #[must_use]
    pub fn signer_address(&self) -> &str {
        &self.context.signer
    }

    /// Construct a runtime, rejecting missing adapters and zero concurrency.
    ///
    /// # Errors
    ///
    /// Returns an error when concurrency is zero or no adapters are enabled.
    pub fn new(
        store: Arc<dyn StateStore>,
        gateway: Arc<dyn ChainGateway>,
        policy: Arc<dyn Policy>,
        adapters: impl IntoIterator<Item = (ActionKind, Arc<dyn ActionAdapter>)>,
        settings: RuntimeSettings,
        faults: Arc<dyn FaultInjector>,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, CookerError> {
        if settings.max_concurrency == 0 {
            return Err(CookerError::InvalidConfig(
                "runtime concurrency must be positive".to_owned(),
            ));
        }
        let adapters: BTreeMap<_, _> = adapters.into_iter().collect();
        if adapters.is_empty() {
            return Err(CookerError::InvalidConfig(
                "runtime requires at least one adapter".to_owned(),
            ));
        }
        Ok(Self {
            store,
            gateway,
            policy,
            adapters,
            context: settings.adapter_context,
            faults,
            clock,
            max_concurrency: settings.max_concurrency,
        })
    }

    /// Execute one owned lease through simulation, submission, and observation.
    ///
    /// # Errors
    ///
    /// Returns an error when durable invariants, policy reads, adapter preparation, or an
    /// injected checkpoint prevents the lifecycle from reaching a classified result.
    #[allow(
        clippy::too_many_lines,
        reason = "the lifecycle keeps persistence and fault checkpoints visibly ordered"
    )]
    pub async fn execute(&self, lease: &ActionLease) -> Result<ExecutionResult, CookerError> {
        self.faults
            .check(ExecutionCheckpoint::AfterIntentPersistence)?;
        let mut action = self.store.get_action(&lease.action_id)?;
        let state = self.store.get_action_state(&lease.action_id)?;
        if state == ActionState::Simulated {
            let prepared = self.store.get_prepared(&lease.action_id)?.ok_or_else(|| {
                CookerError::Store("simulated action has no signed bytes".to_owned())
            })?;
            let adapter = self.adapter_for(&action)?;
            return self
                .submit_simulated(lease, &action, adapter.as_ref(), prepared)
                .await;
        }
        if state != ActionState::Planned {
            return Err(CookerError::InvalidTransition {
                from: state,
                to: ActionState::Simulated,
            });
        }

        let durable_prepared = self.store.get_prepared(&lease.action_id)?;
        let signer_balance = self.gateway.native_balance(&self.context.signer).await?;
        let usage = self
            .store
            .budget_usage(action.agent_id, action.scheduled_at)?;
        match self.policy.evaluate(
            &action,
            signer_balance,
            usage.spent_today_lamports,
            usage.spent_lifetime_lamports,
        )? {
            PolicyDecision::Allow => {}
            PolicyDecision::Delay { until, reason } => {
                return self.delay_planned(lease, until, reason);
            }
            PolicyDecision::Rewrite { payload, reason } => {
                if durable_prepared.is_some() {
                    return self.reject_planned(
                        lease,
                        &action,
                        format!("rewrite_after_preparation:{reason}"),
                    );
                }
                action = self
                    .store
                    .rewrite_action(lease, &payload, self.clock.now(), &reason)?;
                match self.policy.evaluate(
                    &action,
                    signer_balance,
                    usage.spent_today_lamports,
                    usage.spent_lifetime_lamports,
                )? {
                    PolicyDecision::Allow => {}
                    PolicyDecision::Delay { until, reason } => {
                        return self.delay_planned(lease, until, reason);
                    }
                    PolicyDecision::Rewrite { reason, .. } => {
                        return self.reject_planned(
                            lease,
                            &action,
                            format!("rewrite_did_not_converge:{reason}"),
                        );
                    }
                    PolicyDecision::Reject { reason } => {
                        return self.reject_planned(lease, &action, reason);
                    }
                }
            }
            PolicyDecision::Reject { reason } => {
                return self.reject_planned(lease, &action, reason);
            }
        }

        let adapter = self.adapter_for(&action)?;
        let prepared = if let Some(prepared) = durable_prepared {
            prepared
        } else {
            let prepared = adapter.prepare(&self.context, &action).await?;
            if prepared.action_id != action.id {
                return Err(CookerError::Chain(
                    "adapter returned a transaction for another action".to_owned(),
                ));
            }
            self.store
                .record_prepared(lease, &prepared, self.clock.now())?;
            self.faults
                .check(ExecutionCheckpoint::AfterPreparedPersistence)?;
            prepared
        };
        self.simulate_planned(lease, &action, adapter.as_ref(), prepared)
            .await
    }

    fn adapter_for(&self, action: &PlannedAction) -> Result<Arc<dyn ActionAdapter>, CookerError> {
        self.adapters
            .get(&action.payload.kind())
            .filter(|adapter| adapter.supports(&action.payload))
            .cloned()
            .ok_or_else(|| {
                CookerError::Policy(format!(
                    "no enabled adapter supports {:?}",
                    action.payload.kind()
                ))
            })
    }

    fn delay_planned(
        &self,
        lease: &ActionLease,
        until: chrono::DateTime<chrono::Utc>,
        reason: String,
    ) -> Result<ExecutionResult, CookerError> {
        self.store
            .defer_action(lease, until, self.clock.now(), &reason)?;
        Ok(ExecutionResult::Delayed { reason })
    }

    fn reject_planned(
        &self,
        lease: &ActionLease,
        action: &PlannedAction,
        reason: String,
    ) -> Result<ExecutionResult, CookerError> {
        let now = self.clock.now();
        let trace = terminal_trace(
            action,
            None,
            now,
            TraceOutcome::Rejected,
            &self.context.signer,
            [("reason".to_owned(), reason.clone())],
        );
        self.store.transition_action_with_trace(
            lease,
            ActionState::Planned,
            ActionState::Rejected,
            now,
            Some(&reason),
            &trace,
        )?;
        self.store.release_lease(lease, self.clock.now())?;
        Ok(ExecutionResult::Rejected { reason })
    }

    async fn simulate_planned(
        &self,
        lease: &ActionLease,
        action: &PlannedAction,
        adapter: &dyn ActionAdapter,
        prepared: cooker_core::PreparedAction,
    ) -> Result<ExecutionResult, CookerError> {
        let simulation = self.gateway.simulate(&prepared.transaction).await?;
        self.store
            .record_simulation(lease, &simulation, self.clock.now())?;
        if !simulation.succeeded {
            let reason = simulation
                .error
                .unwrap_or_else(|| "transaction simulation failed".to_owned());
            let now = self.clock.now();
            let trace = terminal_trace(
                action,
                None,
                now,
                TraceOutcome::Rejected,
                &self.context.signer,
                [
                    ("reason".to_owned(), reason.clone()),
                    ("stage".to_owned(), "simulation".to_owned()),
                ],
            );
            self.store.transition_action_with_trace(
                lease,
                ActionState::Planned,
                ActionState::Rejected,
                now,
                Some(&reason),
                &trace,
            )?;
            self.store.release_lease(lease, self.clock.now())?;
            return Ok(ExecutionResult::Rejected { reason });
        }
        self.store.transition_action(
            lease,
            ActionState::Planned,
            ActionState::Simulated,
            self.clock.now(),
            None,
        )?;
        self.faults.check(ExecutionCheckpoint::AfterSimulation)?;
        self.submit_simulated(lease, action, adapter, prepared)
            .await
    }

    async fn submit_simulated(
        &self,
        lease: &ActionLease,
        action: &cooker_core::PlannedAction,
        adapter: &dyn ActionAdapter,
        prepared: cooker_core::PreparedAction,
    ) -> Result<ExecutionResult, CookerError> {
        self.store
            .record_submission(lease, &prepared.signature, self.clock.now())?;
        self.store.transition_action(
            lease,
            ActionState::Simulated,
            ActionState::Submitted,
            self.clock.now(),
            None,
        )?;
        self.faults
            .check(ExecutionCheckpoint::AfterSignaturePersistence)?;

        let sent_signature = match self.gateway.submit(&prepared.transaction).await {
            Ok(signature) if signature == prepared.signature => signature,
            Ok(signature) => {
                let reason = format!(
                    "gateway returned signature {signature}, expected {}",
                    prepared.signature
                );
                return self.mark_unknown(lease, Some(prepared.signature), reason);
            }
            Err(error) => {
                return self.mark_unknown(
                    lease,
                    Some(prepared.signature),
                    format!("submission requires reconciliation: {error}"),
                );
            }
        };
        if let Err(error) = self
            .faults
            .check(ExecutionCheckpoint::AfterSendResponseLost)
        {
            return self.mark_unknown(lease, Some(sent_signature), error.to_string());
        }

        let receipt = match adapter.observe(&self.context, action, &prepared).await {
            Ok(receipt) => receipt,
            Err(error) => {
                return self.mark_unknown(
                    lease,
                    Some(sent_signature),
                    format!("post-submission observation failed: {error}"),
                );
            }
        };
        self.promote_receipt(lease, &receipt)
    }

    /// Audit one already claimed confirmed action without resending it.
    ///
    /// # Errors
    ///
    /// Returns an error when signed bytes are absent, lease ownership is invalid, the original
    /// signer adapter is unavailable, observation fails, or the audit cannot be persisted.
    pub async fn audit_confirmation(
        &self,
        lease: &ActionLease,
    ) -> Result<ExecutionResult, CookerError> {
        let action = self.store.get_action(&lease.action_id)?;
        let state = self.store.get_action_state(&lease.action_id)?;
        if state != ActionState::Confirmed {
            return Err(CookerError::InvalidTransition {
                from: state,
                to: ActionState::Orphaned,
            });
        }
        let prepared = self
            .store
            .get_prepared(&lease.action_id)?
            .ok_or_else(|| CookerError::Store("confirmed action has no signed bytes".to_owned()))?;
        let signature = self
            .store
            .get_submission_signature(&lease.action_id)?
            .ok_or_else(|| CookerError::Store("confirmed action has no signature".to_owned()))?;
        if signature != prepared.signature {
            return Err(CookerError::Store(
                "prepared and submitted signatures differ".to_owned(),
            ));
        }
        let adapter = self
            .adapters
            .get(&action.payload.kind())
            .filter(|adapter| adapter.supports(&action.payload))
            .ok_or_else(|| {
                CookerError::Policy("confirmation audit adapter is not enabled".to_owned())
            })?;
        let receipt = adapter.observe(&self.context, &action, &prepared).await?;
        let audited_at = self.clock.now();
        if matches!(
            receipt.status,
            ConfirmationStatus::Confirmed | ConfirmationStatus::Finalized
        ) && receipt.postconditions_met
        {
            self.store
                .complete_confirmation_audit(lease, &receipt, audited_at)?;
            return Ok(ExecutionResult::Audited { signature });
        }

        let reason = match receipt.status {
            ConfirmationStatus::Confirmed | ConfirmationStatus::Finalized => {
                "confirmed transaction postconditions no longer hold".to_owned()
            }
            status => format!(
                "confirmation status downgraded to {}",
                confirmation_name(status)
            ),
        };
        let mut trace = receipt_trace(
            &action,
            &receipt,
            TraceOutcome::Failed,
            &self.context.signer,
        );
        trace
            .attributes
            .insert("correction".to_owned(), "confirmation_orphaned".to_owned());
        trace
            .attributes
            .insert("prior_state".to_owned(), "confirmed".to_owned());
        self.store
            .orphan_confirmation(lease, &receipt, audited_at, &reason, &trace)?;
        Ok(ExecutionResult::Orphaned { signature, reason })
    }

    /// Reconcile one already claimed `Submitted`, `Unknown`, or `Orphaned` action without
    /// resending it.
    ///
    /// # Errors
    ///
    /// Returns an error when signed bytes are absent, lease ownership is invalid, gateway
    /// observation fails, or a durable transition invariant is violated.
    pub async fn reconcile(&self, lease: &ActionLease) -> Result<ExecutionResult, CookerError> {
        let action = self.store.get_action(&lease.action_id)?;
        let mut state = self.store.get_action_state(&lease.action_id)?;
        if !matches!(
            state,
            ActionState::Submitted | ActionState::Unknown | ActionState::Orphaned
        ) {
            return Err(CookerError::InvalidTransition {
                from: state,
                to: ActionState::Unknown,
            });
        }
        let prepared = self
            .store
            .get_prepared(&lease.action_id)?
            .ok_or_else(|| CookerError::Store("submitted action has no signed bytes".to_owned()))?;
        let signature = self
            .store
            .get_submission_signature(&lease.action_id)?
            .ok_or_else(|| CookerError::Store("submitted action has no signature".to_owned()))?;
        if signature != prepared.signature {
            return Err(CookerError::Store(
                "prepared and submitted signatures differ".to_owned(),
            ));
        }
        let adapter = self
            .adapters
            .get(&action.payload.kind())
            .filter(|adapter| adapter.supports(&action.payload))
            .ok_or_else(|| CookerError::Policy("recovery adapter is not enabled".to_owned()))?;
        if state == ActionState::Orphaned {
            self.store.transition_action(
                lease,
                ActionState::Orphaned,
                ActionState::Unknown,
                self.clock.now(),
                Some("reconciling orphaned confirmation without resubmission"),
            )?;
            state = ActionState::Unknown;
        }
        let receipt = adapter.observe(&self.context, &action, &prepared).await?;
        if receipt.status == ConfirmationStatus::Missing
            && self.gateway.block_height().await? > prepared.last_valid_block_height
        {
            if state == ActionState::Submitted {
                self.store.transition_action(
                    lease,
                    ActionState::Submitted,
                    ActionState::Unknown,
                    self.clock.now(),
                    Some("signature absent during reconciliation"),
                )?;
            }
            self.store.record_receipt(lease, &receipt)?;
            let now = self.clock.now();
            let trace = terminal_trace(
                &action,
                Some(signature.clone()),
                now,
                TraceOutcome::Failed,
                &self.context.signer,
                [
                    ("reason".to_owned(), "blockhash_expired".to_owned()),
                    ("status".to_owned(), "missing".to_owned()),
                ],
            );
            self.store.transition_action_with_trace(
                lease,
                ActionState::Unknown,
                ActionState::Expired,
                now,
                Some("blockhash validity window elapsed without a signature"),
                &trace,
            )?;
            self.store.release_lease(lease, self.clock.now())?;
            return Ok(ExecutionResult::Expired { signature });
        }
        self.promote_receipt(lease, &receipt)
    }

    fn promote_receipt(
        &self,
        lease: &ActionLease,
        receipt: &ChainReceipt,
    ) -> Result<ExecutionResult, CookerError> {
        let action = self.store.get_action(&lease.action_id)?;
        let state = self.store.get_action_state(&lease.action_id)?;
        match receipt.status {
            ConfirmationStatus::Confirmed | ConfirmationStatus::Finalized
                if receipt.postconditions_met =>
            {
                let signature = receipt.signature.clone();
                self.store.record_receipt(lease, receipt)?;
                self.faults
                    .check(ExecutionCheckpoint::AfterConfirmationBeforePromotion)?;
                let trace = receipt_trace(
                    &action,
                    receipt,
                    TraceOutcome::Confirmed,
                    &self.context.signer,
                );
                self.store.transition_action_with_trace(
                    lease,
                    state,
                    ActionState::Confirmed,
                    self.clock.now(),
                    None,
                    &trace,
                )?;
                self.store.release_lease(lease, self.clock.now())?;
                Ok(ExecutionResult::Confirmed { signature })
            }
            ConfirmationStatus::Failed => {
                let reason = receipt
                    .error
                    .clone()
                    .unwrap_or_else(|| "network confirmed transaction failure".to_owned());
                self.store.record_receipt(lease, receipt)?;
                let trace =
                    receipt_trace(&action, receipt, TraceOutcome::Failed, &self.context.signer);
                self.store.transition_action_with_trace(
                    lease,
                    state,
                    ActionState::Failed,
                    self.clock.now(),
                    Some(&reason),
                    &trace,
                )?;
                self.store.release_lease(lease, self.clock.now())?;
                Ok(ExecutionResult::Failed { reason })
            }
            _ => {
                let signature = receipt.signature.clone();
                let reason = if receipt.postconditions_met {
                    "transaction has not reached confirmed commitment".to_owned()
                } else {
                    "adapter postconditions are not yet proven".to_owned()
                };
                self.store.record_receipt(lease, receipt)?;
                self.mark_unknown(lease, Some(signature), reason)
            }
        }
    }

    fn mark_unknown(
        &self,
        lease: &ActionLease,
        signature: Option<String>,
        reason: String,
    ) -> Result<ExecutionResult, CookerError> {
        let state = self.store.get_action_state(&lease.action_id)?;
        if state == ActionState::Submitted {
            self.store.transition_action(
                lease,
                ActionState::Submitted,
                ActionState::Unknown,
                self.clock.now(),
                Some(&reason),
            )?;
        } else if state != ActionState::Unknown {
            return Err(CookerError::InvalidTransition {
                from: state,
                to: ActionState::Unknown,
            });
        }
        self.store.release_lease(lease, self.clock.now())?;
        Ok(ExecutionResult::Unknown { signature, reason })
    }
}

fn receipt_trace(
    action: &PlannedAction,
    receipt: &ChainReceipt,
    outcome: TraceOutcome,
    signer: &str,
) -> TraceEvent {
    let mut attributes = BTreeMap::from([
        (
            "status".to_owned(),
            confirmation_name(receipt.status).to_owned(),
        ),
        (
            "postconditions_met".to_owned(),
            receipt.postconditions_met.to_string(),
        ),
        (
            "observation_count".to_owned(),
            receipt.observations.len().to_string(),
        ),
    ]);
    if let Some(slot) = receipt.slot {
        attributes.insert("slot".to_owned(), slot.to_string());
    }
    if let Some(error) = &receipt.error {
        attributes.insert("error".to_owned(), error.clone());
    }
    for (index, observation) in receipt.observations.iter().enumerate() {
        let prefix = format!("observation.{index}");
        attributes.insert(format!("{prefix}.kind"), observation.kind.clone());
        attributes.insert(format!("{prefix}.account"), observation.account.clone());
        if let Some(delta) = observation.expected_delta {
            attributes.insert(format!("{prefix}.delta"), delta.to_string());
        }
        for (key, value) in &observation.attributes {
            attributes.insert(format!("{prefix}.{key}"), value.clone());
        }
    }
    terminal_trace(
        action,
        Some(receipt.signature.clone()),
        receipt.observed_at,
        outcome,
        signer,
        attributes,
    )
}

fn terminal_trace(
    action: &PlannedAction,
    signature: Option<String>,
    observed_at: chrono::DateTime<chrono::Utc>,
    outcome: TraceOutcome,
    signer: &str,
    extra_attributes: impl IntoIterator<Item = (String, String)>,
) -> TraceEvent {
    let mut attributes = BTreeMap::from([("signer".to_owned(), signer.to_owned())]);
    let destination = match &action.payload {
        ActionPayload::NativeTransfer { destination, .. } => {
            attributes.insert("route".to_owned(), "system_program".to_owned());
            Some(destination.clone())
        }
        ActionPayload::SplTransfer {
            mint,
            destination_owner,
            ..
        } => {
            attributes.insert("mint".to_owned(), mint.clone());
            attributes.insert("route".to_owned(), "spl_token".to_owned());
            Some(destination_owner.clone())
        }
        ActionPayload::JupiterSwap {
            input_mint,
            output_mint,
            max_slippage_bps,
            ..
        } => {
            attributes.insert("input_mint".to_owned(), input_mint.clone());
            attributes.insert("output_mint".to_owned(), output_mint.clone());
            attributes.insert("route".to_owned(), format!("{input_mint}->{output_mint}"));
            attributes.insert("max_slippage_bps".to_owned(), max_slippage_bps.to_string());
            Some(output_mint.clone())
        }
        ActionPayload::StakeLifecycle {
            operation,
            stake_account,
            ..
        } => {
            let name = match operation {
                cooker_core::StakeOperation::Enter => "enter",
                cooker_core::StakeOperation::Deactivate => "deactivate",
                cooker_core::StakeOperation::Withdraw => "withdraw",
            };
            attributes.insert("stake_operation".to_owned(), name.to_owned());
            attributes.insert("route".to_owned(), "native_stake".to_owned());
            stake_account.clone()
        }
        ActionPayload::Idle => None,
    };
    attributes.extend(extra_attributes);
    TraceEvent {
        schema_version: TraceEvent::SCHEMA_VERSION,
        run_id: action.run_id,
        agent_id: action.agent_id,
        action_id: action.id.clone(),
        sequence: action.sequence,
        action_kind: action.payload.kind(),
        scheduled_at: action.scheduled_at,
        observed_at: Some(observed_at),
        amount: action.payload.input_amount(),
        destination,
        signature,
        outcome,
        attributes,
    }
}

const fn confirmation_name(status: ConfirmationStatus) -> &'static str {
    match status {
        ConfirmationStatus::Missing => "missing",
        ConfirmationStatus::Processed => "processed",
        ConfirmationStatus::Confirmed => "confirmed",
        ConfirmationStatus::Finalized => "finalized",
        ConfirmationStatus::Failed => "failed",
    }
}
