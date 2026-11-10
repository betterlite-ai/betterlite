mod tests;

use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};

pub trait BudgetClock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;

    // process uptime, the clock LiteLLM's legacy carry cutoff reads from time.monotonic
    fn uptime(&self) -> Duration;
}

pub struct SystemClock {
    pub started: Instant,
}

impl BudgetClock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }

    fn uptime(&self) -> Duration {
        self.started.elapsed()
    }
}
