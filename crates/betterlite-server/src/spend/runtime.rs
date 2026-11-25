mod handles;
pub mod parts;
pub mod pricing;
mod tests;
mod worker;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::spend_queue::{
    SpendQueueError as QueueSettingsError, SpendQueueSettings, load_spend_queue_settings,
};
use betterlite_config::spend_storage::{Settings as SpendStorageSettings, SpendUpdates};
use betterlite_db::queues::pod_lock::PodLockManager;
use betterlite_db::queues::pod_lock::cache::RedisLockCache;
use betterlite_db::redis_spend::buffer::RedisUpdateBuffer;
use betterlite_db::redis_spend::publication::RedisSpendPublisher;
use betterlite_db::spend_flush::monitor::SpendMonitorSettings;
use betterlite_db::spend_flush::service::SpendLogsService;
use betterlite_db::spend_flush::shutdown::{SpendLogsMonitorSlot, drain_spend_logs_queue};
use betterlite_db::spend_log_storage::batch_claim::SpendLogAttachment;
use betterlite_db::spend_log_storage::postgres::PostgresSpendLogsWriter;
use betterlite_db::spend_log_storage::rows::SpendLogQueue;
use betterlite_db::spend_queues::SpendQueueError;
use betterlite_db::spend_queues::entity::SpendUpdateQueue;
use betterlite_db::spend_writer::accounting::{
    AdmissionFuture, SpendAccounting, SpendAccountingAdmission, SpendAccountingClosed,
};
use betterlite_db::spend_writer::settings::{DEFAULT_PROXY_BUDGET_NAME, SpendWriterSettings};
use betterlite_db::spend_writer::{DbSpendUpdateWriter, SpendWriterFailureSink, SpendWriterQueues};
use betterlite_db::tool_discovery::{ToolDiscoveryError, ToolDiscoveryQueue};
use betterlite_db::window_queue::WindowQueueError;
use betterlite_db::window_queue::queue::WindowSpendUpdateQueue;
use tokio::sync::Mutex as WriteLock;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use self::handles::{build_leases, build_queues, build_redis};
use self::parts::{DailyQueues, SpendRuntimeParts, SpendUsageQueues};
use self::worker::spawn_accounting;

#[derive(Debug, thiserror::Error)]
pub enum SpendRuntimeError {
    #[error("the spend accounting queue needs a positive capacity")]
    AccountingCapacity,
    #[error("the spend runtime is already installed")]
    AlreadyInstalled,
    #[error("the spend queue settings could not be read: {0}")]
    QueueSettings(#[from] QueueSettingsError),
    #[error(transparent)]
    Queue(#[from] SpendQueueError),
    #[error(transparent)]
    Discovery(#[from] ToolDiscoveryError),
    #[error(transparent)]
    Window(#[from] WindowQueueError),
}

#[derive(Clone, Debug)]
pub struct SpendRuntimeSettings {
    pub writer: SpendWriterSettings,
    pub queue: SpendQueueSettings,
    pub monitor: SpendMonitorSettings,
    pub log_queue_bytes: usize,
    pub accounting_capacity: usize,
    pub use_redis_transaction_buffer: bool,
    pub dequeue_count: i64,
}

impl SpendRuntimeSettings {
    pub fn from_policy(
        policy: &SpendStorageSettings,
        environment: &BTreeMap<String, String>,
    ) -> Result<Self, SpendRuntimeError> {
        let queue = load_spend_queue_settings(environment)?;
        Ok(Self {
            writer: SpendWriterSettings {
                disable_spend_updates: policy.spend_updates == SpendUpdates::Disabled,
                disable_spend_logs: policy.disable_spend_logs,
                max_budget: policy.max_budget,
                proxy_budget_name: DEFAULT_PROXY_BUDGET_NAME.to_owned(),
            },
            queue,
            monitor: policy.spend_log_monitor,
            log_queue_bytes: policy.spend_log_queue_max_bytes,
            accounting_capacity: usize::try_from(queue.queue_capacity)
                .map_err(|_out_of_range| SpendRuntimeError::AccountingCapacity)?,
            use_redis_transaction_buffer: policy.use_redis_transaction_buffer,
            dequeue_count: policy.max_redis_buffer_dequeue_count,
        })
    }
}

// Every handle DBSpendUpdateWriter and the scheduler share for one server run: queues, Redis buffering, leases and the log monitor.
pub struct SpendRuntime {
    pub(crate) entity: Arc<SpendUpdateQueue>,
    pub(crate) daily: DailyQueues,
    pub(crate) discovery: Arc<ToolDiscoveryQueue>,
    pub(crate) window: Arc<WindowSpendUpdateQueue>,
    pub(crate) logs: Arc<SpendLogQueue>,
    pub(crate) redis_buffer: RedisUpdateBuffer,
    pub(crate) publisher: RedisSpendPublisher,
    pub(crate) leases: Arc<PodLockManager<RedisLockCache>>,
    pub(crate) monitor: SpendLogsMonitorSlot,
    pub(crate) write_lock: WriteLock<()>,
    pub(crate) settings: SpendRuntimeSettings,
    pub(crate) spend_logs: Option<Arc<PostgresSpendLogsWriter>>,
    pub(crate) usage: SpendUsageQueues,
    pub(crate) failures: Arc<dyn SpendWriterFailureSink>,
    pub(crate) sender: Mutex<Option<mpsc::Sender<SpendAccounting>>>,
    pub(crate) tasks: TaskTracker,
    pub(crate) shutdown_signal: CancellationToken,
}

impl SpendRuntime {
    pub fn new(
        settings: SpendRuntimeSettings,
        parts: SpendRuntimeParts,
    ) -> Result<Arc<Self>, SpendRuntimeError> {
        if settings.accounting_capacity == 0 {
            return Err(SpendRuntimeError::AccountingCapacity);
        }
        let queues = build_queues(&settings, &parts.queue_events)?;
        let (redis_buffer, publisher) = build_redis(
            &settings,
            parts.redis.as_ref(),
            Arc::clone(&parts.redis_events),
        );
        let shutdown_signal = CancellationToken::new();
        let (sender, tasks) = spawn_accounting(&settings, &parts, &queues, &shutdown_signal);
        let leases = build_leases(
            &settings,
            parts.redis.as_ref(),
            parts.pod_lock_events,
            parts.pod_id,
        );
        Ok(Arc::new(Self {
            entity: queues.entity,
            daily: queues.daily,
            discovery: queues.discovery,
            window: queues.window,
            logs: queues.logs,
            redis_buffer,
            publisher,
            leases,
            monitor: SpendLogsMonitorSlot::new(),
            write_lock: WriteLock::new(()),
            settings,
            spend_logs: parts.spend_logs,
            usage: parts.usage,
            failures: parts.failures,
            sender: Mutex::new(Some(sender)),
            tasks,
            shutdown_signal,
        }))
    }

    // update_database's request-time writer: the same queues the accounting worker charges, the log claim and the tool discovery.
    #[must_use]
    pub fn writer(&self) -> DbSpendUpdateWriter<'_, PostgresSpendLogsWriter> {
        DbSpendUpdateWriter {
            settings: self.settings.writer.clone(),
            queues: SpendWriterQueues {
                entity: Some(self.entity.as_ref()),
                daily_user: Some(self.daily.user.as_ref()),
                daily_end_user: Some(self.daily.end_user.as_ref()),
                daily_agent: Some(self.daily.agent.as_ref()),
                daily_team: Some(self.daily.team.as_ref()),
                daily_org: Some(self.daily.org.as_ref()),
                daily_tag: Some(self.daily.tag.as_ref()),
                tool_usage: self.usage.tool_usage.as_deref(),
                model_usage: self.usage.model_usage.as_deref(),
                discovery: Some(self.discovery.as_ref()),
                turns: None,
            },
            attachment: self.spend_logs.as_deref().map(|table| SpendLogAttachment {
                table,
                queue: self.logs.as_ref(),
            }),
            failures: self.failures.as_ref(),
            admission: self,
        }
    }

    // The one shutdown owner: no more work is admitted, the accounting worker finishes its queue, then the logs are drained and parked.
    pub async fn shutdown<S>(&self, service: &S) -> Result<(), S::Error>
    where
        S: SpendLogsService,
    {
        self.sender
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        self.shutdown_signal.cancel();
        self.tasks.close();
        self.tasks.wait().await;
        drain_spend_logs_queue(
            &self.monitor,
            &self.write_lock,
            service,
            &self.logs,
            &self.redis_buffer,
        )
        .await
    }
}

impl SpendAccountingAdmission for SpendRuntime {
    fn admit(&self, work: SpendAccounting) -> AdmissionFuture<'_> {
        let sender = self
            .sender
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        Box::pin(async move {
            let Some(open_sender) = sender else {
                return Err(SpendAccountingClosed);
            };
            open_sender
                .send(work)
                .await
                .map_err(|_closed| SpendAccountingClosed)
        })
    }
}
