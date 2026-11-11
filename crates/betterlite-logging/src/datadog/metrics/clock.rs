use std::time::{SystemTime, UNIX_EPOCH};

use rust_decimal::Decimal;

// LiteLLM reads time.time() and datetime.now(), so a clock reports epoch seconds with microseconds
pub trait Clock: Send + Sync {
    fn now_seconds(&self) -> Decimal;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_seconds(&self) -> Decimal {
        let elapsed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let micros = i64::try_from(elapsed.as_micros()).unwrap_or(i64::MAX);
        Decimal::new(micros, 6)
    }
}
