use std::time::{SystemTime, UNIX_EPOCH};

pub trait Clock: Send + Sync {
    fn unix_seconds(&self) -> i64;
}

#[derive(Debug, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn unix_seconds(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|elapsed| i64::try_from(elapsed.as_secs()).ok())
            .unwrap_or_default()
    }
}
