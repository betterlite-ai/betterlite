use std::sync::{Arc, Mutex, PoisonError};

use tokio::sync::Mutex as WriteLock;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::redis_spend::buffer::RedisUpdateBuffer;
use crate::spend_flush::monitor::SpendMonitorSettings;
use crate::spend_flush::monitor::monitor_spend_logs_queue;
use crate::spend_flush::service::SpendLogsService;
use crate::spend_log_storage::rows::{QueuePosition, SpendLogQueue};

mod tests;

// MAX_SPEND_LOG_DRAIN_ITERATIONS: one shutdown runs at most this many job passes before it parks what is left.
pub const MAX_DRAIN_PASSES: u32 = 20;

#[derive(Debug, thiserror::Error)]
pub enum MonitorStartError {
    #[error("a spend logs queue monitor is already running")]
    AlreadyRunning,
}

// The running monitor and the token that stops it, owned by the client's slot until the shutdown takes it out.
struct MonitorHandle {
    shutdown: CancellationToken,
    task: JoinHandle<()>,
}

// spend_logs_queue_monitor_task and spend_log_flush_requested: one monitor per client, started once and cleared by the shutdown drain.
pub struct SpendLogsMonitorSlot {
    tasks: TaskTracker,
    handle: Mutex<Option<MonitorHandle>>,
}

impl SpendLogsMonitorSlot {
    #[must_use]
    pub fn new() -> Self {
        Self {
            tasks: TaskTracker::new(),
            handle: Mutex::new(None),
        }
    }

    // A second start while one monitor runs would leave the first without an owner, so it is refused.
    pub fn start<Service>(
        &self,
        service: Arc<Service>,
        queue: Arc<SpendLogQueue>,
        settings: SpendMonitorSettings,
    ) -> Result<(), MonitorStartError>
    where
        Service: SpendLogsService + 'static,
    {
        let mut handle = self.handle.lock().unwrap_or_else(PoisonError::into_inner);
        if handle.is_some() {
            return Err(MonitorStartError::AlreadyRunning);
        }
        let shutdown = CancellationToken::new();
        let task = self.tasks.spawn(monitor_spend_logs_queue(
            service,
            queue,
            settings,
            shutdown.clone(),
        ));
        *handle = Some(MonitorHandle { shutdown, task });
        Ok(())
    }

    #[must_use]
    pub fn is_running(&self) -> bool {
        self.handle
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
    }

    // The monitor's task and the wait it was parked on end before the write lock is taken, so no pass can wake up inside the drain; the flush signal stays bound as the source's event does.
    async fn stop(&self) {
        let taken = self
            .handle
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        let Some(monitor) = taken else {
            return;
        };
        monitor.shutdown.cancel();
        if let Err(error) = monitor.task.await {
            tracing::warn!(
                "Spend tracking - the spend logs queue monitor ended abnormally: {error}"
            );
        }
    }
}

impl Default for SpendLogsMonitorSlot {
    fn default() -> Self {
        Self::new()
    }
}

// drain_spend_logs_queue: the monitor is joined first, then the shutdown holds the write lock for its passes and parks the rows left over.
pub async fn drain_spend_logs_queue<Service>(
    slot: &SpendLogsMonitorSlot,
    write_lock: &WriteLock<()>,
    service: &Service,
    queue: &SpendLogQueue,
    redis: &RedisUpdateBuffer,
) -> Result<(), Service::Error>
where
    Service: SpendLogsService,
{
    slot.stop().await;
    let _write = write_lock.lock().await;
    let outcome = drain_to_db(service).await;
    park_remaining_spend_logs(queue, redis).await;
    outcome
}

// _drain_spend_logs_queue_to_db: an empty census ends the drain, and twenty full passes leave a warning instead of a longer shutdown.
async fn drain_to_db<Service>(service: &Service) -> Result<(), Service::Error>
where
    Service: SpendLogsService,
{
    for _pass in 0..MAX_DRAIN_PASSES {
        if service.queued_transactions() == 0 {
            return Ok(());
        }
        service.run_spend_logs().await?;
    }
    let remaining = service.queued_transactions();
    if remaining > 0 {
        tracing::error!(
            "Spend tracking - {remaining} spend log rows still queued after {MAX_DRAIN_PASSES} drain passes"
        );
    }
    Ok(())
}

// _park_remaining_spend_logs: request logs only, Redis first, then back to the memory head so the exit loses nothing else silently.
pub async fn park_remaining_spend_logs(queue: &SpendLogQueue, redis: &RedisUpdateBuffer) {
    let rows = queue.dequeue(usize::MAX);
    if rows.is_empty() || redis.park_spend_logs(&rows).await {
        return;
    }
    queue.enqueue(&rows, QueuePosition::Head);
    let lost = rows.len();
    tracing::error!(
        "Spend tracking - {lost} spend log rows could not be written or parked in Redis and will be lost on exit"
    );
}
