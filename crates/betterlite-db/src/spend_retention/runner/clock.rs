use chrono::{DateTime, Utc};
use tokio::time::Instant;

use crate::spend_retention::batches::deadline::MonotonicClock;

// one clock for both readings a run needs: the wall clock picks cutoffs and the monotonic one bounds the budget
pub trait CleanupClock: MonotonicClock {
    fn utc_now(&self) -> DateTime<Utc>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemCleanupClock;

impl MonotonicClock for SystemCleanupClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

impl CleanupClock for SystemCleanupClock {
    fn utc_now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}
