pub mod clock;
pub mod credentials;
pub mod encryption;
pub mod flush;
pub mod health;
pub mod message;
pub mod sender;
pub mod settings;
pub mod signing;

pub(crate) mod tests;

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

use async_trait::async_trait;
use tokio_util::task::TaskTracker;

use crate::batch::delivery::{BatchCancelled, BatchDelivery};
use crate::batch::logger::BatchQueue;
use crate::callback_logger::CallbackLogger;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::exporter_options::base64_strip::Base64Stripper;
use crate::exporter_options::health::IntegrationHealthStatus;
use crate::fields::payload::StandardLoggingPayload;
use crate::http_analytics_loggers::delivery::Delivery;

use self::clock::Clock;
use self::credentials::{CredentialError, CredentialSource};
use self::health::HealthPayloadFactory;
use self::sender::SqsSender;
use self::settings::Settings;

pub const CALLBACK_NAME: &str = "aws_sqs";

pub struct SqsDeps<D> {
    pub delivery: D,
    pub credentials: Arc<dyn CredentialSource>,
    pub clock: Arc<dyn Clock>,
    pub health_payload: Arc<dyn HealthPayloadFactory>,
    pub tasks: TaskTracker,
}

pub struct SqsLogger<D> {
    sender: Arc<SqsSender<D>>,
    pub(crate) queue: BatchQueue<StandardLoggingPayload>,
    tasks: TaskTracker,
    stripper: Base64Stripper,
}

#[derive(Debug, thiserror::Error)]
pub enum SqsError {
    #[error("the SQS credentials cannot be used")]
    Credentials(#[from] CredentialError),
    #[error("the SQS base64 redaction pattern could not be compiled")]
    Pattern(#[source] regex::Error),
}

impl<D> SqsLogger<D>
where
    D: Delivery + 'static,
{
    pub fn new(settings: Settings, deps: SqsDeps<D>) -> Result<Self, SqsError> {
        deps.credentials.validate(&settings.credentials)?;
        let queue = BatchQueue::new(settings.batch);
        let stripper = Base64Stripper::new().map_err(SqsError::Pattern)?;
        Ok(Self {
            sender: Arc::new(SqsSender {
                settings,
                delivery: deps.delivery,
                credentials: deps.credentials,
                clock: deps.clock,
                health_payload: deps.health_payload,
            }),
            queue,
            tasks: deps.tasks,
            stripper,
        })
    }

    // LiteLLM's async_send_batch only creates one task per queued payload and clears nothing
    #[must_use]
    pub fn async_send_batch(self: &Arc<Self>) -> usize {
        let payloads = self.queue.pending();
        let launched = payloads.len();
        for payload in payloads {
            self.spawn_message(payload);
        }
        launched
    }

    pub async fn flush_queue(self: &Arc<Self>) {
        let batch = SqsBatchDelivery {
            logger: Arc::clone(self),
        };
        let report = self.queue.flush(&batch).await;
        tracing::debug!(
            batch = report.batch,
            delivered = report.delivered,
            "sqs logger - sent a batch"
        );
    }

    pub async fn async_health_check(&self) -> IntegrationHealthStatus {
        match self.sender.health_payload.build() {
            Ok(dummy) => {
                self.sender.async_send_message(&dummy).await;
                IntegrationHealthStatus::healthy()
            }
            Err(error) => IntegrationHealthStatus::unhealthy(error.to_string()),
        }
    }

    fn spawn_message(&self, payload: StandardLoggingPayload) {
        let sender = Arc::clone(&self.sender);
        self.tasks.spawn(async move {
            sender.async_send_message(&payload).await;
        });
    }

    fn enqueue(&self, payload: &StandardLoggingPayload) {
        let queued_payload = if self.sender.settings.strip_base64_files {
            self.stripper
                .strip_base64_from_messages(payload, self.sender.settings.strip_max_depth)
        } else {
            payload.clone()
        };
        let queued = self.queue.enqueue(queued_payload);
        tracing::debug!(
            queue_length = queued.depth,
            batch_size = self.queue.settings.batch_size,
            "sqs logging: queue length and batch size"
        );
    }
}

#[async_trait]
impl<D> CallbackLogger for SqsLogger<D>
where
    D: Delivery + 'static,
{
    fn callback_name(&self) -> &str {
        CALLBACK_NAME
    }

    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.enqueue(event.payload.as_ref());
        Ok(())
    }

    async fn async_log_failure_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.enqueue(event.payload.as_ref());
        Ok(())
    }
}

struct SqsBatchDelivery<D> {
    logger: Arc<SqsLogger<D>>,
}

impl<D> BatchDelivery<StandardLoggingPayload> for SqsBatchDelivery<D>
where
    D: Delivery + 'static,
{
    fn deliver(
        &self,
        batch: &[StandardLoggingPayload],
        _settled: &AtomicUsize,
    ) -> impl Future<
        Output = Result<Vec<StandardLoggingPayload>, BatchCancelled<StandardLoggingPayload>>,
    > + Send {
        for payload in batch {
            self.logger.spawn_message(payload.clone());
        }
        async { Ok(Vec::new()) }
    }
}
