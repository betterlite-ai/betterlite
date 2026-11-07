use std::time::Duration;

use tokio::time::Instant;

pub(super) struct TimeoutSummary {
    pub(super) window: Duration,
    pub(super) burst: usize,
    pub(super) last_callback: Option<String>,
    pub(super) deadline: Option<Instant>,
}

impl TimeoutSummary {
    pub(super) fn record(&mut self, callback: String) {
        self.burst = self.burst.saturating_add(1);
        self.last_callback = Some(callback);
        if self.deadline.is_none() {
            self.deadline = Instant::now()
                .checked_add(self.window)
                .or_else(|| Some(Instant::now()));
        }
    }

    pub(super) fn emit(&mut self, timeout: Duration, total: usize) {
        let callbacks = self.burst;
        if callbacks == 0 {
            return;
        }
        let callback = self.last_callback.take().unwrap_or_default();
        let timeout_milliseconds = u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX);
        tracing::warn!(
            callbacks,
            callback,
            timeout_milliseconds,
            total,
            "logging callbacks timed out and released their permits"
        );
        self.burst = 0;
        self.deadline = None;
    }
}
