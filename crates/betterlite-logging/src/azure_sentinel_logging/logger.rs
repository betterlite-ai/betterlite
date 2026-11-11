use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde::Serialize;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::batch::logger::{BatchQueue, FlushReport};
use crate::callback_logger::event::LoggerEvent;
use crate::callback_logger::{CallbackLogger, error::LoggerError};
use crate::datadog_logging::clock::Clock;
use crate::datadog_logging::transport::Transport;
use crate::fields::payload::StandardLoggingPayload;

use super::audit::StandardAuditLogPayload;
use super::delivery::SentinelBatch;
use super::settings::SentinelSettings;
use super::token::TokenCache;

pub struct SentinelDeps {
    pub transport: Arc<dyn Transport>,
    pub clock: Arc<dyn Clock>,
    pub stopped: CancellationToken,
    // the server owns the flush timer's task; without a tracker only explicit flushes run
    pub tasks: Option<TaskTracker>,
}

// model and audit records queue apart, so a batch never mixes the two schemas
pub struct SentinelStream<E> {
    pub queue: BatchQueue<E>,
    pub awaiting_retry: AtomicBool,
}

impl<E> SentinelStream<E> {
    fn new(settings: &SentinelSettings) -> Self {
        Self {
            queue: BatchQueue::new(settings.batch),
            awaiting_retry: AtomicBool::new(false),
        }
    }
}

pub struct AzureSentinelLogger {
    pub settings: SentinelSettings,
    pub model: SentinelStream<StandardLoggingPayload>,
    pub audit: SentinelStream<StandardAuditLogPayload>,
    pub transport: Arc<dyn Transport>,
    pub clock: Arc<dyn Clock>,
    pub tokens: TokenCache,
    pub last_flush_time: Mutex<Duration>,
    pub stopped: CancellationToken,
}

// the retry flag follows the queue even when the flush future is dropped midway
struct AwaitingRetry<'stream, E> {
    stream: &'stream SentinelStream<E>,
    retried: Option<bool>,
}

impl<E> Drop for AwaitingRetry<'_, E> {
    fn drop(&mut self) {
        let pending = !self
            .stream
            .queue
            .queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty();
        self.stream
            .awaiting_retry
            .store(self.retried.unwrap_or(true) && pending, Ordering::SeqCst);
    }
}

impl AzureSentinelLogger {
    #[must_use]
    pub fn new(settings: SentinelSettings, deps: SentinelDeps) -> Arc<Self> {
        let last_flush_time = deps.clock.now_unix();
        let logger = Arc::new(Self {
            model: SentinelStream::new(&settings),
            audit: SentinelStream::new(&settings),
            settings,
            transport: deps.transport,
            clock: deps.clock,
            tokens: TokenCache::default(),
            last_flush_time: Mutex::new(last_flush_time),
            stopped: deps.stopped,
        });
        if let Some(tasks) = deps.tasks {
            let periodic = Arc::clone(&logger);
            tasks.spawn(async move { periodic.periodic_flush().await });
        }
        logger
    }

    // a payload the call did not carry is warned about and left out of the queue
    pub async fn async_log_model_event(&self, candidate: Option<&StandardLoggingPayload>) {
        let Some(payload) = candidate else {
            tracing::warn!("Azure Sentinel: standard_logging_object not found in kwargs");
            return;
        };
        self.enqueue(&self.model, &self.settings.model_url, payload.clone())
            .await;
    }

    pub async fn async_log_audit_log_event(&self, audit_log: StandardAuditLogPayload) {
        self.enqueue(&self.audit, &self.settings.audit_url, audit_log)
            .await;
    }

    async fn enqueue<E>(&self, stream: &SentinelStream<E>, url: &str, record: E)
    where
        E: Serialize + Clone + Send + Sync,
    {
        let enqueued = stream.queue.enqueue(record);
        if enqueued.depth >= self.settings.batch.batch_size
            && !stream.awaiting_retry.load(Ordering::SeqCst)
        {
            self.send_stream(stream, url).await;
        }
    }

    async fn send_stream<E>(&self, stream: &SentinelStream<E>, url: &str) -> FlushReport
    where
        E: Serialize + Clone + Send + Sync,
    {
        let mut awaiting = AwaitingRetry {
            stream,
            retried: None,
        };
        let report = stream
            .queue
            .flush(&SentinelBatch { logger: self, url })
            .await;
        if !report.cancelled {
            awaiting.retried = Some(report.requeued > 0);
        }
        report
    }

    // the model batch goes first, then the audit batch, as LiteLLM's flush_queue does
    pub async fn flush_queue(&self) {
        self.send_stream(&self.model, &self.settings.model_url)
            .await;
        self.send_stream(&self.audit, &self.settings.audit_url)
            .await;
        *self
            .last_flush_time
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = self.clock.now_unix();
    }

    async fn periodic_flush(&self) {
        while timeout(self.settings.batch.flush_interval, self.stopped.cancelled())
            .await
            .is_err()
        {
            self.flush_queue().await;
        }
    }
}

#[async_trait::async_trait]
impl CallbackLogger for AzureSentinelLogger {
    fn callback_name(&self) -> &'static str {
        "azure_sentinel"
    }

    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.async_log_model_event(Some(&event.payload)).await;
        Ok(())
    }

    async fn async_log_failure_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.async_log_model_event(Some(&event.payload)).await;
        Ok(())
    }
}
