//! Stable, bounded priority scheduling without one task per wallet.

use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet, BinaryHeap},
};

use chrono::{DateTime, Utc};

use crate::{domain::AgentId, error::CookerError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct QueueStamp {
    due_at: DateTime<Utc>,
    ticket: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct QueueEntry {
    due_at: DateTime<Utc>,
    ticket: u64,
    agent_id: AgentId,
}

impl Ord for QueueEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse each field because BinaryHeap is a max-heap. Tickets provide
        // deterministic FIFO order for equal deadlines; completed agents get
        // fresh tickets and therefore rotate behind existing peers.
        other
            .due_at
            .cmp(&self.due_at)
            .then_with(|| other.ticket.cmp(&self.ticket))
            .then_with(|| other.agent_id.cmp(&self.agent_id))
    }
}

impl PartialOrd for QueueEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// One due agent admitted into the bounded worker set.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Dispatch {
    /// Agent selected for one planner decision.
    pub agent_id: AgentId,
    /// Deadline that made the agent eligible.
    pub due_at: DateTime<Utc>,
}

/// Single priority queue with stable ties, capacity, and in-flight accounting.
#[derive(Debug)]
pub struct StableScheduler {
    heap: BinaryHeap<QueueEntry>,
    queued: BTreeMap<AgentId, QueueStamp>,
    in_flight: BTreeSet<AgentId>,
    capacity: usize,
    max_in_flight: usize,
    next_ticket: u64,
    cancelled: bool,
}

impl StableScheduler {
    /// Create a scheduler with explicit total-agent and worker backpressure.
    pub fn new(capacity: usize, max_in_flight: usize) -> Result<Self, CookerError> {
        if capacity == 0 || max_in_flight == 0 || max_in_flight > capacity {
            return Err(CookerError::InvalidConfig(
                "scheduler requires 0 < max_in_flight <= capacity".to_owned(),
            ));
        }
        Ok(Self {
            heap: BinaryHeap::with_capacity(capacity),
            queued: BTreeMap::new(),
            in_flight: BTreeSet::new(),
            capacity,
            max_in_flight,
            next_ticket: 0,
            cancelled: false,
        })
    }

    /// Insert or reschedule a queued agent.
    pub fn enqueue(&mut self, agent_id: AgentId, due_at: DateTime<Utc>) -> Result<(), CookerError> {
        if self.cancelled {
            return Err(CookerError::LeaseConflict(
                "scheduler is cancelled".to_owned(),
            ));
        }
        if self.in_flight.contains(&agent_id) {
            return Err(CookerError::LeaseConflict(format!(
                "agent {agent_id} is already in flight"
            )));
        }
        let is_new = !self.queued.contains_key(&agent_id);
        if is_new && self.len() >= self.capacity {
            return Err(CookerError::LeaseConflict(
                "scheduler capacity reached".to_owned(),
            ));
        }
        let ticket = self.next_ticket;
        self.next_ticket = self
            .next_ticket
            .checked_add(1)
            .ok_or_else(|| CookerError::Store("scheduler ticket overflow".to_owned()))?;
        let stamp = QueueStamp { due_at, ticket };
        self.queued.insert(agent_id, stamp);
        self.heap.push(QueueEntry {
            due_at,
            ticket,
            agent_id,
        });
        self.compact_if_needed();
        Ok(())
    }

    /// Admit at most `limit` due agents without exceeding worker capacity.
    #[must_use]
    pub fn dispatch_due(&mut self, now: DateTime<Utc>, limit: usize) -> Vec<Dispatch> {
        if self.cancelled || limit == 0 {
            return Vec::new();
        }
        let available = self.max_in_flight.saturating_sub(self.in_flight.len());
        let target = available.min(limit);
        let mut dispatched = Vec::with_capacity(target);
        while dispatched.len() < target {
            let Some(entry) = self.heap.peek().copied() else {
                break;
            };
            if entry.due_at > now {
                break;
            }
            self.heap.pop();
            let current = self.queued.get(&entry.agent_id);
            if current
                != Some(&QueueStamp {
                    due_at: entry.due_at,
                    ticket: entry.ticket,
                })
            {
                continue;
            }
            self.queued.remove(&entry.agent_id);
            self.in_flight.insert(entry.agent_id);
            dispatched.push(Dispatch {
                agent_id: entry.agent_id,
                due_at: entry.due_at,
            });
        }
        dispatched
    }

    /// Complete one admitted decision and optionally schedule its next deadline.
    pub fn complete(
        &mut self,
        agent_id: AgentId,
        next_due_at: Option<DateTime<Utc>>,
    ) -> Result<(), CookerError> {
        if !self.in_flight.remove(&agent_id) {
            return Err(CookerError::LeaseConflict(format!(
                "agent {agent_id} is not in flight"
            )));
        }
        if !self.cancelled
            && let Some(due_at) = next_due_at
        {
            self.enqueue(agent_id, due_at)?;
        }
        Ok(())
    }

    /// Stop all future dispatches and discard the in-memory waiting queue.
    pub fn cancel(&mut self) {
        self.cancelled = true;
        self.heap.clear();
        self.queued.clear();
    }

    /// Earliest active deadline, skipping stale reschedule entries.
    pub fn next_due(&mut self) -> Option<DateTime<Utc>> {
        loop {
            let entry = self.heap.peek().copied()?;
            if self.queued.get(&entry.agent_id)
                == Some(&QueueStamp {
                    due_at: entry.due_at,
                    ticket: entry.ticket,
                })
            {
                return Some(entry.due_at);
            }
            self.heap.pop();
        }
    }

    /// Number of queued and admitted agents.
    #[must_use]
    pub fn len(&self) -> usize {
        self.queued.len() + self.in_flight.len()
    }

    /// Whether no agent is queued or admitted.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Number of admitted agents occupying worker slots.
    #[must_use]
    pub fn in_flight_len(&self) -> usize {
        self.in_flight.len()
    }

    /// Whether cancellation has been requested.
    #[must_use]
    pub const fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    fn compact_if_needed(&mut self) {
        let threshold = self.capacity.saturating_mul(2).max(1);
        if self.heap.len() <= threshold {
            return;
        }
        self.heap = self
            .queued
            .iter()
            .map(|(agent_id, stamp)| QueueEntry {
                due_at: stamp.due_at,
                ticket: stamp.ticket,
                agent_id: *agent_id,
            })
            .collect();
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeDelta, TimeZone};
    use proptest::prelude::*;
    use uuid::Uuid;

    use super::*;

    fn agent(value: u128) -> AgentId {
        AgentId(Uuid::from_u128(value))
    }

    #[test]
    fn equal_deadlines_are_fifo_and_completed_agents_rotate() {
        let now = Utc.with_ymd_and_hms(2026, 7, 17, 12, 0, 0).unwrap();
        let mut scheduler = StableScheduler::new(3, 1).unwrap();
        scheduler.enqueue(agent(3), now).unwrap();
        scheduler.enqueue(agent(1), now).unwrap();
        scheduler.enqueue(agent(2), now).unwrap();

        let first = scheduler.dispatch_due(now, 3);
        assert_eq!(first[0].agent_id, agent(3));
        scheduler.complete(agent(3), Some(now)).unwrap();
        let second = scheduler.dispatch_due(now, 3);
        assert_eq!(second[0].agent_id, agent(1));
        scheduler.complete(agent(1), None).unwrap();
        let third = scheduler.dispatch_due(now, 3);
        assert_eq!(third[0].agent_id, agent(2));
    }

    #[test]
    fn reschedule_discards_stale_heap_entry() {
        let now = Utc.with_ymd_and_hms(2026, 7, 17, 12, 0, 0).unwrap();
        let later = now + TimeDelta::hours(1);
        let mut scheduler = StableScheduler::new(1, 1).unwrap();
        scheduler.enqueue(agent(1), now).unwrap();
        scheduler.enqueue(agent(1), later).unwrap();
        assert!(scheduler.dispatch_due(now, 1).is_empty());
        assert_eq!(scheduler.next_due(), Some(later));
    }

    #[test]
    fn capacity_and_worker_backpressure_are_independent() {
        let now = Utc.with_ymd_and_hms(2026, 7, 17, 12, 0, 0).unwrap();
        let mut scheduler = StableScheduler::new(3, 2).unwrap();
        for value in 1..=3 {
            scheduler.enqueue(agent(value), now).unwrap();
        }
        assert!(scheduler.enqueue(agent(4), now).is_err());
        assert_eq!(scheduler.dispatch_due(now, 10).len(), 2);
        assert_eq!(scheduler.in_flight_len(), 2);
        assert_eq!(scheduler.dispatch_due(now, 10).len(), 0);
    }

    #[test]
    fn cancellation_stops_new_dispatches_and_requeues() {
        let now = Utc.with_ymd_and_hms(2026, 7, 17, 12, 0, 0).unwrap();
        let mut scheduler = StableScheduler::new(2, 1).unwrap();
        scheduler.enqueue(agent(1), now).unwrap();
        let dispatch = scheduler.dispatch_due(now, 1);
        assert_eq!(dispatch.len(), 1);
        scheduler.cancel();
        scheduler.complete(agent(1), Some(now)).unwrap();
        assert!(scheduler.dispatch_due(now, 1).is_empty());
        assert!(scheduler.is_empty());
    }

    #[test]
    fn repeated_rescheduling_compacts_stale_entries() {
        let now = Utc.with_ymd_and_hms(2026, 7, 17, 12, 0, 0).unwrap();
        let mut scheduler = StableScheduler::new(2, 1).unwrap();
        for offset in 0..100 {
            scheduler
                .enqueue(agent(1), now + TimeDelta::seconds(offset))
                .unwrap();
        }
        assert!(scheduler.heap.len() <= 4);
        assert_eq!(scheduler.len(), 1);
        assert_eq!(scheduler.next_due(), Some(now + TimeDelta::seconds(99)));
    }

    proptest! {
        #[test]
        fn arbitrary_due_times_dispatch_in_stable_chronological_order(
            offsets in prop::collection::vec(-86_400_i64..=86_400, 1..128)
        ) {
            let now = Utc.with_ymd_and_hms(2026, 7, 17, 12, 0, 0).unwrap();
            let mut scheduler = StableScheduler::new(offsets.len(), offsets.len()).unwrap();
            for (index, offset) in offsets.iter().copied().enumerate() {
                scheduler
                    .enqueue(agent(index as u128 + 1), now + TimeDelta::seconds(offset))
                    .unwrap();
            }

            let dispatched = scheduler.dispatch_due(now, offsets.len() + 1);
            let actual: Vec<_> = dispatched
                .iter()
                .map(|item| {
                    let index = usize::try_from(item.agent_id.0.as_u128() - 1).unwrap();
                    (offsets[index], index)
                })
                .collect();
            let mut expected: Vec<_> = offsets
                .iter()
                .copied()
                .enumerate()
                .filter_map(|(index, offset)| (offset <= 0).then_some((offset, index)))
                .collect();
            expected.sort_unstable();

            prop_assert_eq!(actual, expected);
            prop_assert_eq!(scheduler.in_flight_len(), dispatched.len());
            prop_assert!(scheduler.in_flight_len() <= offsets.len());
        }
    }
}
