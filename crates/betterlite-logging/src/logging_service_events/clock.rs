use chrono::{DateTime, FixedOffset, Utc};

use crate::logging_success::ports::SuccessClock;

pub struct SystemClock;

impl SuccessClock for SystemClock {
    fn now(&self) -> DateTime<FixedOffset> {
        Utc::now().fixed_offset()
    }
}
