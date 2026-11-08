use std::time::{Duration, SystemTime, UNIX_EPOCH};

// LiteLLM reads time.time(), so a clock reports seconds since the Unix epoch; a Duration keeps the arithmetic exact.
pub trait Clock: Send + Sync {
    fn now_unix(&self) -> Duration;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_unix(&self) -> Duration {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
    }
}
