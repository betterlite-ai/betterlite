use std::time::{Instant, SystemTime, UNIX_EPOCH};

use thiserror::Error;

#[derive(Debug, Error)]
#[error("the system clock is too far from the Unix epoch to count in whole seconds")]
pub struct ClockRangeError;

pub trait HubClock: Send + Sync {
    fn monotonic_seconds(&self) -> u64;
    fn unix_seconds(&self) -> Result<i64, ClockRangeError>;
}

#[derive(Debug)]
pub struct SystemHubClock {
    started: Instant,
}

impl Default for SystemHubClock {
    fn default() -> Self {
        Self {
            started: Instant::now(),
        }
    }
}

impl HubClock for SystemHubClock {
    fn monotonic_seconds(&self) -> u64 {
        self.started.elapsed().as_secs()
    }

    fn unix_seconds(&self) -> Result<i64, ClockRangeError> {
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(since) => i64::try_from(since.as_secs()).map_err(|_overflow| ClockRangeError),
            Err(error) => i64::try_from(error.duration().as_secs())
                .ok()
                .and_then(i64::checked_neg)
                .ok_or(ClockRangeError),
        }
    }
}
