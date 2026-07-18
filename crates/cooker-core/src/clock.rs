//! Injectable wall and virtual clocks.

use std::sync::{Arc, RwLock};

use chrono::{DateTime, TimeDelta, Utc};

use crate::error::CookerError;

/// Clock boundary used by deterministic scheduling.
pub trait Clock: Send + Sync {
    /// Return the current UTC time.
    fn now(&self) -> DateTime<Utc>;
}

/// Production wall clock.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// Thread-safe clock advanced explicitly by tests and simulations.
#[derive(Clone, Debug)]
pub struct VirtualClock {
    now: Arc<RwLock<DateTime<Utc>>>,
}

impl VirtualClock {
    /// Start at a fixed UTC timestamp.
    #[must_use]
    pub fn new(now: DateTime<Utc>) -> Self {
        Self {
            now: Arc::new(RwLock::new(now)),
        }
    }

    /// Move forward to a timestamp, rejecting time travel.
    pub fn advance_to(&self, next: DateTime<Utc>) -> Result<(), CookerError> {
        let mut current = self
            .now
            .write()
            .map_err(|_| CookerError::Store("virtual clock lock poisoned".to_owned()))?;
        if next < *current {
            return Err(CookerError::InvalidConfig(
                "virtual clock cannot move backwards".to_owned(),
            ));
        }
        *current = next;
        Ok(())
    }

    /// Move forward by a non-negative duration.
    pub fn advance_by(&self, delta: TimeDelta) -> Result<(), CookerError> {
        if delta < TimeDelta::zero() {
            return Err(CookerError::InvalidConfig(
                "virtual clock delta cannot be negative".to_owned(),
            ));
        }
        self.advance_to(self.now() + delta)
    }
}

impl Clock for VirtualClock {
    fn now(&self) -> DateTime<Utc> {
        match self.now.read() {
            Ok(now) => *now,
            Err(poisoned) => *poisoned.into_inner(),
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    #[test]
    fn virtual_clock_is_monotonic() {
        let start = Utc.with_ymd_and_hms(2026, 7, 16, 12, 0, 0).unwrap();
        let clock = VirtualClock::new(start);
        clock.advance_by(TimeDelta::minutes(5)).unwrap();
        assert_eq!(clock.now(), start + TimeDelta::minutes(5));
        assert!(clock.advance_to(start).is_err());
    }
}
