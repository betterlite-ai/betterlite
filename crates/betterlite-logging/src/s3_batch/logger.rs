use crate::s3_batch::Element;
use crate::s3_batch::flush::{
    FlushContext, FlushError, FlushSettings, MonotonicClock, UploadOperation, send_batch,
};
use crate::s3_batch::logger::setup::{schedule, transport, unsupported_flow};
use crate::s3_batch::queue::{BatchQueue, QueueState};
use crate::s3_batch::upload_bounded::{BoundsError, UPLOAD_WAIT, UploadLimiter};
use crate::s3_transport::error::S3TransportError;
use crate::s3_transport::signing::Clock;
use crate::s3_transport::transport::{S3HttpClient, S3Transport, ThrottleSink, UploadOutcome};
use crate::s3_v2::{Feature, PartitionGranularityCache, S3Parameters};
use betterlite_config::loader::EnvironmentLookup;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Mutex as AsyncMutex;
use tokio::time::{Instant, timeout};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
pub(crate) const CALLBACK_NAME: &str = "s3_v2";
pub struct S3BatchDeps {
    pub client: Arc<dyn S3HttpClient>,
    pub clock: Arc<dyn Clock>,
    pub monotonic: Arc<dyn MonotonicClock>,
    pub environment: Arc<dyn EnvironmentLookup + Send + Sync>,
    pub throttle_observer: Option<Arc<dyn ThrottleSink>>,
    pub cold_storage_logger: Option<String>,
    pub tasks: Option<TaskTracker>,
    pub shutdown: CancellationToken,
}
#[derive(Debug, thiserror::Error)]
pub enum InitError {
    #[error(transparent)]
    Endpoint(#[from] S3TransportError),
    #[error(transparent)]
    Limiter(#[from] BoundsError),
    #[error(
        "the S3 logger credential setting '{setting}' needs an AWS profile, role or web-identity flow; set s3_aws_access_key_id and s3_aws_secret_access_key instead"
    )]
    CredentialFlow { setting: &'static str },
    #[error(
        "the S3 logger has no AWS credentials; set s3_aws_access_key_id and s3_aws_secret_access_key or AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY"
    )]
    MissingCredentials,
}
pub struct S3BatchLogger {
    pub parameters: S3Parameters<()>,
    pub queue: BatchQueue,
    pub settings: FlushSettings,
    pub batch_size: usize,
    pub flush_interval: Duration,
    pub partition_cache: Mutex<PartitionGranularityCache>,
    pub environment: Arc<dyn EnvironmentLookup + Send + Sync>,
    pub clock: Arc<dyn Clock>,
    pub monotonic: Arc<dyn MonotonicClock>,
    pub transport: S3Transport,
    pub uploads: Arc<UploadLimiter>,
    pub flush_lock: AsyncMutex<()>,
    pub stopped: CancellationToken,
    pub in_flight_flushes: AtomicUsize,
}
struct TransportUpload<'logger> {
    logger: &'logger S3BatchLogger,
}
struct FlushScope<'logger> {
    logger: &'logger S3BatchLogger,
}
impl Drop for FlushScope<'_> {
    fn drop(&mut self) {
        self.logger.in_flight_flushes.fetch_sub(1, Ordering::SeqCst);
    }
}
impl UploadOperation for TransportUpload<'_> {
    fn upload<'operation>(
        &'operation self,
        element: &'operation Element,
    ) -> Pin<Box<dyn Future<Output = UploadOutcome> + Send + 'operation>> {
        Box::pin(self.logger.upload_bounded(element))
    }
}
impl S3BatchLogger {
    pub fn new(parameters: S3Parameters<()>, deps: S3BatchDeps) -> Result<Arc<Self>, InitError> {
        if let Some(setting) = unsupported_flow(&parameters.connection.credentials) {
            return Err(InitError::CredentialFlow { setting });
        }
        let (transport, uploads) = transport(&parameters, &deps)?;
        let (batch_size, flush_interval) = schedule(deps.environment.as_ref());
        let settings = FlushSettings {
            max_retry_age: parameters.max_retry_age_seconds,
            max_queue_size: parameters.max_queue_size,
            batch_file_upload: parameters.features.contains(&Feature::BatchFileUpload),
            cold_storage_logger: deps.cold_storage_logger,
            max_concurrent_uploads: parameters.max_concurrent_uploads,
        };
        let logger = Arc::new(Self {
            queue: BatchQueue {
                state: Mutex::new(QueueState::default()),
                max_queue_size: settings.max_queue_size,
            },
            parameters,
            settings,
            batch_size,
            flush_interval,
            partition_cache: Mutex::new(PartitionGranularityCache::default()),
            environment: deps.environment,
            clock: deps.clock,
            monotonic: deps.monotonic,
            transport,
            uploads,
            flush_lock: AsyncMutex::new(()),
            stopped: deps.shutdown,
            in_flight_flushes: AtomicUsize::new(0),
        });
        if let Some(tasks) = deps.tasks {
            let periodic = Arc::clone(&logger);
            tasks.spawn(async move { periodic.periodic_flush().await });
        }
        Ok(logger)
    }
    pub async fn async_send_batch(&self) -> Result<usize, FlushError> {
        let context = FlushContext {
            queue: &self.queue,
            settings: &self.settings,
            clock: self.clock.as_ref(),
            monotonic: self.monotonic.as_ref(),
        };
        send_batch(&context, &TransportUpload { logger: self }).await
    }
    pub async fn flush_queue(&self) -> Result<(), FlushError> {
        let _flush = self.flush_lock.lock().await;
        match self.async_send_batch().await {
            Ok(batch_len) => {
                self.queue.finish_success(batch_len);
                Ok(())
            }
            Err(error) => {
                let dropped = self.queue.trim_overflow();
                tracing::error!(
                    %error,
                    dropped,
                    "CustomLogger: async_send_batch raised; preserving events in queue for retry"
                );
                Err(error)
            }
        }
    }
    async fn upload_bounded(&self, element: &Element) -> UploadOutcome {
        let Some(deadline) = Instant::now().checked_add(UPLOAD_WAIT) else {
            return UploadOutcome::Retry;
        };
        self.in_flight_flushes.fetch_add(1, Ordering::SeqCst);
        let _scope = FlushScope { logger: self };
        match self.uploads.acquire(deadline).await {
            Ok(permit) => {
                let outcome = self.transport.async_upload_data_to_s3(element).await;
                drop(permit);
                outcome
            }
            Err(error) => {
                tracing::warn!(
                    %error,
                    key = %element.s3_object_key,
                    "s3 logging: upload did not run"
                );
                UploadOutcome::Retry
            }
        }
    }
    async fn periodic_flush(self: Arc<Self>) {
        while timeout(self.flush_interval, self.stopped.cancelled())
            .await
            .is_err()
        {
            tracing::debug!(interval = ?self.flush_interval, "CustomLogger periodic flush");
            if let Err(error) = self.flush_queue().await {
                tracing::debug!(%error, "the periodic S3 flush kept its records for the next attempt");
            }
        }
    }
}
mod event;
mod setup;
mod tests;
