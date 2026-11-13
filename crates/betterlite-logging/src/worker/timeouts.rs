use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::sync::Notify;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;

use super::counters::Counters;

#[derive(Debug, Default)]
pub(super) struct TimeoutBurst {
    burst: AtomicUsize,
    armed: AtomicBool,
    last_callback: Mutex<String>,
    notify: Notify,
}

impl TimeoutBurst {
    pub(super) fn record(&self, callback: String) {
        self.burst.fetch_add(1, Ordering::SeqCst);
        *self
            .last_callback
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = callback;
        if !self.armed.swap(true, Ordering::SeqCst) {
            self.notify.notify_one();
        }
    }

    fn emit(&self, counters: &Counters, timeout: Duration) {
        self.armed.store(false, Ordering::SeqCst);
        let callbacks = self.burst.swap(0, Ordering::SeqCst);
        if callbacks == 0 {
            return;
        }
        let callback = self
            .last_callback
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let timeout_milliseconds = u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX);
        let total = counters.timed_out.load(Ordering::SeqCst);
        tracing::warn!(
            callbacks,
            callback,
            timeout_milliseconds,
            total,
            "logging callbacks timed out and released their permits"
        );
    }
}

pub(super) async fn summarize(
    burst: Arc<TimeoutBurst>,
    counters: Arc<Counters>,
    window: Duration,
    timeout: Duration,
    shutdown: CancellationToken,
) {
    loop {
        let Some(()) = shutdown.run_until_cancelled(burst.notify.notified()).await else {
            burst.emit(&counters, timeout);
            return;
        };
        let slept = shutdown.run_until_cancelled(sleep(window)).await;
        burst.emit(&counters, timeout);
        if slept.is_none() {
            return;
        }
    }
}
