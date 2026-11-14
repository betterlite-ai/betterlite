pub mod clock;
pub mod delivery;
pub mod settings;

mod tests;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::loader::EnvironmentLookup;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::argilla_logging::logger::RandomSource;
use crate::batch::logger::{BatchQueue, Enqueued};
use crate::callback_logger::CallbackLogger;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::http_analytics_loggers::delivery::Delivery;
use crate::langsmith_payloads::credentials::{CredentialError, credentials_for_request};
use crate::langsmith_payloads::ids::IdSource;
use crate::langsmith_payloads::prepare::{PayloadError, Prepare, prepare_log_data};
use crate::langsmith_payloads::sampling::{SamplingError, sampling_rate_for_request};

use self::clock::Clock;
use self::delivery::{LangsmithBatch, QueuedRun};
use self::settings::LangsmithSettings;

pub const CALLBACK_NAME: &str = "langsmith";

#[derive(Clone, Copy)]
enum EventKind {
    Success,
    Failure,
}

#[derive(Debug, thiserror::Error)]
pub enum LangsmithError {
    #[error("the LangSmith sampling rate could not be read")]
    Sampling(#[from] SamplingError),
    #[error("the LangSmith credentials could not be read")]
    Credentials(#[from] CredentialError),
    #[error("the LangSmith run could not be built")]
    Payload(#[from] PayloadError),
}

pub struct LangsmithDeps<D, R> {
    pub delivery: D,
    pub environment: Arc<dyn EnvironmentLookup + Send + Sync>,
    pub random: R,
    pub ids: Arc<dyn IdSource>,
    pub clock: Arc<dyn Clock>,
    pub tasks: TaskTracker,
    pub shutdown: CancellationToken,
}

pub struct LangsmithLogger<D, R> {
    settings: LangsmithSettings,
    environment: Arc<dyn EnvironmentLookup + Send + Sync>,
    delivery: Arc<D>,
    random: R,
    ids: Arc<dyn IdSource>,
    clock: Arc<dyn Clock>,
    queue: BatchQueue<QueuedRun>,
    tasks: TaskTracker,
}

impl<D, R> LangsmithLogger<D, R>
where
    D: Delivery + 'static,
    R: RandomSource + 'static,
{
    pub fn new(settings: LangsmithSettings, deps: LangsmithDeps<D, R>) -> Arc<Self> {
        let queue = BatchQueue::new(settings.batch);
        let logger = Arc::new(Self {
            settings,
            environment: deps.environment,
            delivery: Arc::new(deps.delivery),
            random: deps.random,
            ids: deps.ids,
            clock: deps.clock,
            queue,
            tasks: deps.tasks,
        });
        logger.spawn_periodic_flush(deps.shutdown);
        logger
    }

    // a sync event cannot await the send, so LiteLLM schedules it and so does the tracker
    fn spawn_flush(&self) {
        let queue = self.queue.clone();
        let delivery = Arc::clone(&self.delivery);
        self.tasks.spawn(async move {
            let batch = LangsmithBatch { delivery };
            let _report = queue.flush(&batch).await;
        });
    }

    fn spawn_periodic_flush(&self, shutdown: CancellationToken) {
        self.tasks.spawn(periodic_flush(
            self.queue.clone(),
            Arc::clone(&self.delivery),
            self.settings.batch.flush_interval,
            shutdown,
        ));
    }

    // the async success and failure events differ only in the message they log
    async fn log_async(&self, event: &LoggerEvent, kind: EventKind) {
        let failure = match kind {
            EventKind::Success => "Langsmith Layer Error - error logging async success event.",
            EventKind::Failure => "Langsmith Layer Error - error logging async failure event.",
        };
        if self.record(event, failure) {
            self.flush_queue().await;
        }
    }

    pub async fn flush_queue(&self) {
        let batch = LangsmithBatch {
            delivery: Arc::clone(&self.delivery),
        };
        let report = self.queue.flush(&batch).await;
        tracing::debug!(
            batch = report.batch,
            delivered = report.delivered,
            "Langsmith batch flushed"
        );
    }

    // logs the event and answers whether the queue reached the batch size; an error never reaches the caller
    fn record(&self, event: &LoggerEvent, failure: &'static str) -> bool {
        match self.try_record(event) {
            Ok(enqueued) => {
                enqueued.is_some_and(|queued| queued.depth >= self.settings.batch.batch_size)
            }
            Err(error) => {
                tracing::error!(%error, "{failure}");
                false
            }
        }
    }

    // the sampling, credentials and run building every LangSmith event shares
    fn try_record(&self, event: &LoggerEvent) -> Result<Option<Enqueued>, LangsmithError> {
        let sampling_rate = sampling_rate_for_request(
            self.settings.sampling_rate,
            Some(event.dynamic_params.as_ref()),
        )?;
        let random_sample = self.random.sample();
        if random_sample > sampling_rate {
            tracing::info!(
                "Skipping Langsmith logging. Sampling rate={sampling_rate}, random_sample={random_sample}"
            );
            return Ok(None);
        }
        let credentials = credentials_for_request(
            self.environment.as_ref(),
            &self.settings.default_credentials,
            Some(event.dynamic_params.as_ref()),
        )?;
        let data = prepare_log_data(
            &Prepare {
                standard_logging_object: Some(event.payload.as_ref()),
                litellm_metadata: event.litellm_metadata.as_ref(),
                credentials: &credentials,
                default_run_name: &self.settings.default_run_name,
                redact_user_api_key_info: self.settings.redact_user_api_key_info,
            },
            self.clock.now(),
            self.ids.as_ref(),
        )?;
        let queued = QueuedRun {
            data,
            api_key: credentials.api_key.clone(),
            project: credentials.project,
            base_url: credentials.base_url,
            tenant_id: credentials.tenant_id,
        };
        Ok(Some(self.queue.enqueue(queued)))
    }
}

#[async_trait]
impl<D, R> CallbackLogger for LangsmithLogger<D, R>
where
    D: Delivery + 'static,
    R: RandomSource + 'static,
{
    fn callback_name(&self) -> &str {
        CALLBACK_NAME
    }

    fn log_success_event(&self, event: &LoggerEvent) -> Result<(), LoggerError> {
        if self.record(event, "Langsmith Layer Error - log_success_event error") {
            self.spawn_flush();
        }
        Ok(())
    }

    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.log_async(&event, EventKind::Success).await;
        Ok(())
    }

    async fn async_log_failure_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.log_async(&event, EventKind::Failure).await;
        Ok(())
    }
}

// CustomBatchLogger.periodic_flush, on the tracker the catalog owns and stopped by its shutdown token
async fn periodic_flush<D>(
    queue: BatchQueue<QueuedRun>,
    delivery: Arc<D>,
    interval: Duration,
    shutdown: CancellationToken,
) where
    D: Delivery + 'static,
{
    loop {
        let _waited = timeout(interval, shutdown.cancelled()).await;
        if shutdown.is_cancelled() {
            break;
        }
        tracing::debug!(?interval, "Langsmith periodic flush");
        let batch = LangsmithBatch {
            delivery: Arc::clone(&delivery),
        };
        let _report = queue.flush(&batch).await;
    }
}
