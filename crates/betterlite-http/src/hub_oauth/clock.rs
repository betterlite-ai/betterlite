use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub trait HubClock: Send + Sync {
    fn monotonic_seconds(&self) -> u64;
    fn unix_seconds(&self) -> i64;
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

    fn unix_seconds(&self) -> i64 {
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(since) => i64::try_from(since.as_secs()).unwrap_or(i64::MAX),
            Err(error) => i64::try_from(error.duration().as_secs())
                .unwrap_or(i64::MAX)
                .saturating_neg(),
        }
    }
}
