use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Notify;
use tokio::task::yield_now;
use tokio::time::{sleep, timeout};
use tokio_util::sync::CancellationToken;

use crate::spend_flush::service::SpendLogsService;
use crate::spend_log_storage::rows::SpendLogQueue;

mod tests;

// SPEND_LOG_QUEUE_SIZE_THRESHOLD and SPEND_LOG_QUEUE_POLL_INTERVAL of litellm/constants.py, loaded once with the rest of the configuration.
pub const DEFAULT_QUEUE_SIZE_THRESHOLD: i64 = 100;
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(2);

// max_backoff: the source stops growing the interval here.
pub const MAX_BACKOFF: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpendMonitorSettings {
    pub queue_size_threshold: i64,
    pub base_interval: Duration,
}

impl SpendMonitorSettings {
    pub const DEFAULT: Self = Self {
        queue_size_threshold: DEFAULT_QUEUE_SIZE_THRESHOLD,
        base_interval: DEFAULT_POLL_INTERVAL,
    };
}

// _monitor_spend_logs_queue: parked rows and the full census are read every pass, and the wait between passes ends early on a flush request.
pub async fn monitor_spend_logs_queue<Service>(
    service: Arc<Service>,
    queue: Arc<SpendLogQueue>,
    settings: SpendMonitorSettings,
    shutdown: CancellationToken,
) where
    Service: SpendLogsService + 'static,
{
    let signal = Arc::new(Notify::new());
    // The source binds its flush event to the client for the monitor's life and never unbinds it, so a request after the last pass reaches nothing.
    queue.set_flush_signal(Some(Arc::clone(&signal)));
    tracing::info!(
        "Starting spend logs queue monitor (threshold: {}, poll_interval: {:?})",
        settings.queue_size_threshold,
        settings.base_interval
    );
    let mut interval = settings.base_interval;
    loop {
        let pass = monitor_pass(&*service, settings, &mut interval, &signal);
        if shutdown.run_until_cancelled(pass).await.is_none() {
            return;
        }
    }
}

// One pass, then the wait that follows it: the error path sleeps with the increased backoff instead of waiting on the flush signal.
async fn monitor_pass<Service>(
    service: &Service,
    settings: SpendMonitorSettings,
    interval: &mut Duration,
    signal: &Notify,
) where
    Service: SpendLogsService,
{
    let outcome = process_queue(service, settings, interval).await;
    match outcome {
        Ok(()) => {
            if wait_for_flush_request(signal, *interval).await {
                *interval = settings.base_interval;
            }
        }
        Err(error) => {
            tracing::error!("Error in spend logs queue monitor: {error}");
            *interval = back_off(*interval);
            sleep(*interval).await;
        }
    }
}

// A nonempty queue runs one pass even below the threshold, which only decides whether the interval resets or grows.
async fn process_queue<Service>(
    service: &Service,
    settings: SpendMonitorSettings,
    interval: &mut Duration,
) -> Result<(), Service::Error>
where
    Service: SpendLogsService,
{
    service.recover_parked_logs().await;
    let queued = service.queued_transactions();
    if queued <= 0 {
        // An empty queue still resets nothing here; the baseline accounting flush joins this branch in B1699.
        *interval = back_off(*interval);
        return Ok(());
    }
    if queued >= settings.queue_size_threshold {
        tracing::debug!(
            "Spend logs queue size ({queued}) reached threshold ({}), triggering processing",
            settings.queue_size_threshold
        );
        *interval = settings.base_interval;
    } else {
        tracing::debug!(
            "Spend logs queue size ({queued}) below threshold ({}), processing with backoff",
            settings.queue_size_threshold
        );
        *interval = back_off(*interval);
    }
    service.update_spend_logs().await
}

// _wait_for_spend_log_flush_request: True when a request woke the wait, and a zero interval yields instead of spinning.
#[must_use]
pub async fn wait_for_flush_request(signal: &Notify, interval: Duration) -> bool {
    if interval.is_zero() {
        yield_now().await;
        return false;
    }
    timeout(interval, signal.notified()).await.is_ok()
}

// min(current_interval * 1.5, max_backoff): whole nanoseconds keep every step exact.
fn back_off(current: Duration) -> Duration {
    let grown = current.as_nanos().saturating_mul(3).saturating_div(2);
    u64::try_from(grown)
        .map_or(MAX_BACKOFF, Duration::from_nanos)
        .min(MAX_BACKOFF)
}
