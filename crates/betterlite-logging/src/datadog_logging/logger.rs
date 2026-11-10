use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::batch::logger::BatchQueue;
use crate::callback_logger::event::LoggerEvent;
use crate::callback_logger::{CallbackLogger, error::LoggerError};
use crate::datadog::apm::NullTracer;
use crate::datadog::payloads::types::DatadogPayload;

use super::batch;
use super::clock::Clock;
use super::error::DatadogError;
use super::settings::{DatadogInit, DatadogSettings};
use super::transport::Transport;

pub type IdGenerator = fn() -> String;

#[must_use]
pub fn uuid_id() -> String {
    // the legacy v1 payload is the only place an id is invented, as LiteLLM does with uuid4
    uuid::Uuid::new_v4().to_string()
}

pub struct DatadogDeps {
    pub transport: Arc<dyn Transport>,
    pub clock: Arc<dyn Clock>,
    pub ids: IdGenerator,
    // the server owns the flush timer's task; without a tracker only explicit flushes run
    pub tasks: Option<TaskTracker>,
}

pub struct DatadogLogger {
    pub settings: DatadogSettings,
    pub queue: BatchQueue<DatadogPayload>,
    pub transport: Arc<dyn Transport>,
    pub clock: Arc<dyn Clock>,
    pub tracer: NullTracer,
    pub ids: IdGenerator,
    pub last_flush_time: Mutex<Duration>,
    pub stopped: CancellationToken,
}

impl DatadogLogger {
    pub fn new(
        init: &DatadogInit,
        environment: &dyn EnvironmentLookup,
        deps: DatadogDeps,
    ) -> Result<Arc<Self>, DatadogError> {
        let settings = DatadogSettings::resolve(init, environment)?;
        let last_flush_time = deps.clock.now_unix();
        let logger = Arc::new(Self {
            queue: BatchQueue::new(settings.batch),
            settings,
            transport: deps.transport,
            clock: deps.clock,
            tracer: NullTracer,
            ids: deps.ids,
            last_flush_time: Mutex::new(last_flush_time),
            stopped: CancellationToken::new(),
        });
        match deps.tasks {
            Some(tasks) => {
                let periodic = Arc::clone(&logger);
                let tracker = tasks.clone();
                let _task =
                    tasks.spawn(async move { batch::periodic_flush(&periodic, &tracker).await });
            }
            None => {
                tracing::debug!("Datadog: no task tracker, the periodic flush did not start");
            }
        }
        Ok(logger)
    }
}

#[async_trait::async_trait]
impl CallbackLogger for DatadogLogger {
    fn callback_name(&self) -> &'static str {
        "datadog"
    }

    fn turn_off_message_logging(&self) -> bool {
        self.settings.options.turn_off_message_logging
    }

    fn message_logging(&self) -> bool {
        self.settings.options.message_logging
    }

    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        batch::log_async_event(self, &event).await;
        Ok(())
    }

    async fn async_log_failure_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        batch::log_async_event(self, &event).await;
        Ok(())
    }
}
