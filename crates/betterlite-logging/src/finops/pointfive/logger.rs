pub mod client;
pub mod core;
pub mod error;
pub mod flush;
pub mod pending;
pub mod record;
pub mod send;

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::sections::pointfive_params::PointFiveInitParams;
use tokio::runtime::Handle;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use self::client::UploadClientSource;
use self::core::PointFiveCore;
use self::error::PointFiveError;
use crate::callback_logger::CallbackLogger;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::exporter_options::health::IntegrationHealthStatus;
use crate::finops::pointfive::upload::PointFiveUploadClient;

pub const CALLBACK_NAME: &str = "pointfive";

pub struct PointFiveDeps {
    pub source: Arc<dyn UploadClientSource>,
    pub given_upload_client: Option<PointFiveUploadClient>,
    pub global_message_redaction: bool,
    pub start_periodic_flush: bool,
    pub tasks: TaskTracker,
    pub shutdown: CancellationToken,
}

pub struct PointFiveLogger {
    pub core: Arc<PointFiveCore>,
    pub tasks: TaskTracker,
    pub shutdown: CancellationToken,
    pub periodic_task: Mutex<Option<JoinHandle<()>>>,
    pub batch_task: Mutex<Option<JoinHandle<()>>>,
}

impl PointFiveLogger {
    pub fn new(
        params: PointFiveInitParams,
        deps: PointFiveDeps,
    ) -> Result<Arc<Self>, PointFiveError> {
        if deps.given_upload_client.is_none() {
            // __init__ refuses to start without a key, rather than at the first flush
            deps.source.upload_client(&params)?;
        }
        let logger = Arc::new(Self {
            core: Arc::new(PointFiveCore::new(
                params,
                deps.given_upload_client,
                deps.source,
                deps.global_message_redaction,
            )),
            tasks: deps.tasks,
            shutdown: deps.shutdown,
            periodic_task: Mutex::new(None),
            batch_task: Mutex::new(None),
        });
        if deps.start_periodic_flush {
            logger.start_periodic_flush_task();
        }
        Ok(logger)
    }

    // _start_periodic_flush_task: started only once an event loop is actually running
    pub fn start_periodic_flush_task(&self) {
        let Ok(handle) = Handle::try_current() else {
            return;
        };
        let mut task = self
            .periodic_task
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let core = Arc::clone(&self.core);
        let interval = Duration::from_secs(core.params.flush_interval);
        let shutdown = self.shutdown.clone();
        *task = Some(self.tasks.spawn_on(
            async move { flush::periodic_flush(&core, interval, shutdown).await },
            &handle,
        ));
    }

    // _flush_task_is_alive: a task whose loop has closed never runs again, and is_finished covers it
    #[must_use]
    pub fn flush_task_is_alive(&self) -> bool {
        self.periodic_task
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .is_some_and(|task| !task.is_finished())
    }

    // _start_batch_flush_task: upload in the background, so no request waits on pointfive
    pub fn start_batch_flush_task(&self) {
        let Ok(handle) = Handle::try_current() else {
            return;
        };
        let mut task = self
            .batch_task
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if task.as_ref().is_some_and(|running| !running.is_finished()) {
            return;
        }
        let core = Arc::clone(&self.core);
        *task = Some(self.tasks.spawn_on(
            async move { flush::flush_queue(&core, true).await },
            &handle,
        ));
    }

    // _enqueue: buffer one record, flushing early once the batch threshold is reached
    pub fn enqueue(&self, event: &LoggerEvent) {
        if !self.flush_task_is_alive() {
            self.start_periodic_flush_task();
        }
        let record = match record::record_for(&self.core, event) {
            Ok(record) => record,
            Err(error) => {
                tracing::error!(%error, "pointfive: failed to queue an event");
                return;
            }
        };
        self.core.queue.push(record);
        self.core.drop_overflow();
        if self.core.queue.len() >= self.core.params.batch_size {
            self.start_batch_flush_task();
        }
    }

    // async_health_check: ask the api whether it accepts this key
    #[must_use]
    pub async fn async_health_check(&self) -> IntegrationHealthStatus {
        let client = match self.core.upload_client() {
            Ok(client) => client,
            Err(error) => return IntegrationHealthStatus::unhealthy(error.to_string()),
        };
        match client.ping().await {
            Ok(()) => IntegrationHealthStatus::healthy(),
            Err(failure) => IntegrationHealthStatus::unhealthy(failure.detail),
        }
    }
}

#[async_trait]
impl CallbackLogger for PointFiveLogger {
    fn callback_name(&self) -> &'static str {
        CALLBACK_NAME
    }

    fn turn_off_message_logging(&self) -> bool {
        self.core.params.turn_off_message_logging
    }

    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.enqueue(&event);
        Ok(())
    }

    async fn async_log_failure_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.enqueue(&event);
        Ok(())
    }
}
