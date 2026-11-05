use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub trait HubClock: Send + Sync {
    fn monotonic_seconds(&self) -> u64;
    fn unix_seconds(&self) -> i64;
}

#[derive(Debug)]
pub struct SystemHubClock {
    started: Instant,
}

impl SystemHubClock {
    #[must_use]
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
        }
    }
}

impl Default for SystemHubClock {
    fn default() -> Self {
        Self::new()
    }
}

impl HubClock for SystemHubClock {
    fn monotonic_seconds(&self) -> u64 {
        self.started.elapsed().as_secs()
    }

    fn unix_seconds(&self) -> i64 {
        let elapsed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        i64::try_from(elapsed).unwrap_or(i64::MAX)
    }
}
