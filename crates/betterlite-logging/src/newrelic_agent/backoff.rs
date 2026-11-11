use std::sync::Mutex;
use std::sync::PoisonError;
use std::time::Duration;

pub const METRIC_WINDOW: Duration = Duration::from_hours(27);

// LiteLLM's class-level _last_metric_emission_time and _metric_lock, shared by every logger a factory builds
#[derive(Default)]
pub struct MetricBackoff {
    last_emission: Mutex<Duration>,
}

impl MetricBackoff {
    #[must_use]
    pub fn is_due(&self, now: Duration) -> bool {
        let last = *self
            .last_emission
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        now.saturating_sub(last) >= METRIC_WINDOW
    }

    // The re-check under the lock is the double-checked one, so two threads cannot both emit
    #[must_use]
    pub fn claim(&self, now: Duration) -> bool {
        let mut last = self
            .last_emission
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if now.saturating_sub(*last) < METRIC_WINDOW {
            return false;
        }
        *last = now;
        true
    }
}
