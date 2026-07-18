//! Bounded task orchestration over already claimed action leases.

use std::{collections::VecDeque, sync::Arc};

use cooker_core::{ActionLease, CookerError};
use tokio::{sync::watch, task::JoinSet};

use crate::{ExecutionResult, RuntimeEngine};

/// Aggregate result returned after the bounded batch settles.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WorkerSummary {
    /// Actions confirmed with observed postconditions.
    pub confirmed: usize,
    /// Prior confirmations successfully re-verified.
    pub audited: usize,
    /// Prior confirmations corrected to orphaned.
    pub orphaned: usize,
    /// Actions rejected by policy or simulation.
    pub rejected: usize,
    /// Actions delayed without execution.
    pub delayed: usize,
    /// Actions requiring reconciliation.
    pub unknown: usize,
    /// Actions proven expired.
    pub expired: usize,
    /// Actions confirmed failed.
    pub failed: usize,
    /// Worker errors that did not produce a lifecycle result.
    pub errors: Vec<String>,
    /// Highest number of simultaneously spawned action workers.
    pub peak_workers: usize,
    /// Claimed leases released without starting after the kill switch closed.
    pub released_without_start: usize,
}

impl RuntimeEngine {
    /// Settle a finite claimed batch with at most configured concurrency.
    pub async fn run_claimed(
        self: Arc<Self>,
        leases: Vec<ActionLease>,
        mut stop: watch::Receiver<bool>,
    ) -> WorkerSummary {
        let mut pending: VecDeque<_> = leases.into();
        let mut workers = JoinSet::new();
        let mut summary = WorkerSummary::default();
        let mut stop_open = true;

        loop {
            while workers.len() < self.max_concurrency && !*stop.borrow() {
                let Some(lease) = pending.pop_front() else {
                    break;
                };
                let runtime = Arc::clone(&self);
                workers.spawn(async move { runtime.execute(&lease).await });
                summary.peak_workers = summary.peak_workers.max(workers.len());
            }

            if workers.is_empty() {
                break;
            }
            tokio::select! {
                changed = stop.changed(), if stop_open => {
                    if changed.is_err() {
                        stop_open = false;
                    }
                }
                joined = workers.join_next() => {
                    if let Some(result) = joined {
                        match result {
                            Ok(result) => record_result(&mut summary, result),
                            Err(error) => summary.errors.push(format!("worker task failed: {error}")),
                        }
                    }
                }
            }
        }

        // Leases not started after cancellation are released for a later controller pass.
        for lease in pending {
            if let Err(error) = self.store.release_lease(&lease, self.clock.now()) {
                summary.errors.push(error.to_string());
            }
        }
        while let Some(result) = workers.join_next().await {
            match result {
                Ok(result) => record_result(&mut summary, result),
                Err(error) => summary.errors.push(format!("worker task failed: {error}")),
            }
        }
        summary
    }
}

pub(crate) fn record_result(
    summary: &mut WorkerSummary,
    result: Result<ExecutionResult, CookerError>,
) {
    match result {
        Ok(ExecutionResult::Confirmed { .. }) => summary.confirmed += 1,
        Ok(ExecutionResult::Audited { .. }) => summary.audited += 1,
        Ok(ExecutionResult::Orphaned { .. }) => summary.orphaned += 1,
        Ok(ExecutionResult::Rejected { .. }) => summary.rejected += 1,
        Ok(ExecutionResult::Delayed { .. }) => summary.delayed += 1,
        Ok(ExecutionResult::Unknown { .. }) => summary.unknown += 1,
        Ok(ExecutionResult::Expired { .. }) => summary.expired += 1,
        Ok(ExecutionResult::Failed { .. }) => summary.failed += 1,
        Err(error) => summary.errors.push(error.to_string()),
    }
}
