use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::time::{Instant, sleep};

use crate::admission_control::in_flight::InFlightRequests;

mod tests;

const POLL_INTERVAL: Duration = Duration::from_millis(100);
const LOG_INTERVAL: Duration = Duration::from_secs(5);

pub trait InFlightCounter: Send + Sync {
    fn in_flight_requests(&self) -> u64;
}

impl InFlightCounter for InFlightRequests {
    fn in_flight_requests(&self) -> u64 {
        self.stats().current
    }
}

#[derive(Debug, Default)]
struct Lifecycle {
    shutting_down: bool,
    drain_performed: bool,
}

// GracefulShutdownManager: one owner per worker, built at startup instead of held in a class attribute
pub struct ShutdownState {
    in_flight: Arc<dyn InFlightCounter>,
    lifecycle: Mutex<Lifecycle>,
    timeout: Duration,
}

impl ShutdownState {
    #[must_use]
    pub fn new(in_flight: Arc<dyn InFlightCounter>, timeout: Duration) -> Self {
        Self {
            in_flight,
            lifecycle: Mutex::new(Lifecycle::default()),
            timeout,
        }
    }

    #[must_use]
    pub fn is_shutting_down(&self) -> bool {
        self.lifecycle
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .shutting_down
    }

    // Idempotent: a SIGTERM after a preStop drain keeps the first start time and logs nothing
    pub fn start_shutdown(&self) {
        {
            let mut lifecycle = self
                .lifecycle
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if lifecycle.shutting_down {
                return;
            }
            lifecycle.shutting_down = true;
        }
        let in_flight_requests = self.in_flight.in_flight_requests();
        tracing::info!("graceful_shutdown_started in_flight_requests={in_flight_requests}");
    }

    pub async fn wait_for_drain(&self, timeout: Option<Duration>, exclude_self: bool) -> u64 {
        if !self.claim_drain() {
            return 0;
        }
        let wait = timeout.unwrap_or(self.timeout);
        // the /health/drain request flows through the in-flight middleware, so it counts itself
        let target = if exclude_self { 1 } else { 0 };
        let initial = self.in_flight.in_flight_requests();
        if wait.is_zero() {
            return initial.saturating_sub(target);
        }
        self.poll_drain(initial, target, wait).await
    }

    // a preStop drain and the lifespan drain both wait; only the first one may spend the window
    fn claim_drain(&self) -> bool {
        let mut lifecycle = self
            .lifecycle
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if lifecycle.drain_performed {
            return false;
        }
        lifecycle.drain_performed = true;
        true
    }

    async fn poll_drain(&self, initial: u64, target: u64, wait: Duration) -> u64 {
        let start = Instant::now();
        let mut last_log = start;
        loop {
            let current = self.in_flight.in_flight_requests();
            if current <= target {
                let drained = initial.saturating_sub(current);
                let elapsed_s = start.elapsed().as_secs_f64();
                tracing::info!(
                    "graceful_shutdown_complete drained_requests={drained} elapsed_s={elapsed_s:.2}"
                );
                return drained;
            }
            let elapsed = start.elapsed();
            if elapsed >= wait {
                let elapsed_s = elapsed.as_secs_f64();
                let timeout_s = wait.as_secs_f64();
                tracing::warn!(
                    "graceful_shutdown_timeout in_flight_requests={current} elapsed_s={elapsed_s:.2} timeout_s={timeout_s:?} — proceeding with teardown"
                );
                return initial.saturating_sub(current);
            }
            let now = Instant::now();
            if now.saturating_duration_since(last_log) >= LOG_INTERVAL {
                let elapsed_s = elapsed.as_secs_f64();
                tracing::info!(
                    "drain_waiting in_flight_requests={current} elapsed_s={elapsed_s:.2}"
                );
                last_log = now;
            }
            sleep(POLL_INTERVAL).await;
        }
    }
}
