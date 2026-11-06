use std::time::{SystemTime, UNIX_EPOCH};

use crate::copilot::device_auth::sleeper::TokioSleeper;

pub trait Clock {
    fn now(&self) -> i64;
}

impl Clock for TokioSleeper {
    fn now(&self) -> i64 {
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(elapsed) => i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX),
            Err(elapsed) => i64::try_from(elapsed.duration().as_secs())
                .ok()
                .and_then(i64::checked_neg)
                .unwrap_or(i64::MIN),
        }
    }
}
