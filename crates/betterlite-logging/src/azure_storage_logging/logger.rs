use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::loader::EnvironmentLookup;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::batch::logger::{BatchQueue, FlushReport};
use crate::callback_logger::CallbackLogger;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::datadog_logging::clock::Clock;
use crate::fields::payload::StandardLoggingPayload;
use crate::private_json::error::PrivateJsonError;

use super::filename::IdSource;
use super::settings::AzureStorageSettings;
use super::signing::SigningError;
use super::token::{ChainProviderFactory, ChainTokenProvider, TokenError, TokenState};
use super::transport::{StorageTransport, StorageTransportError};

mod upload;

pub const CALLBACK_NAME: &str = "azure_storage";
// LiteLLM's _DEFAULT_TTL_FOR_HTTPX_CLIENTS, the lifetime of a cached DataLakeServiceClient
pub const SERVICE_CLIENT_TTL: Duration = Duration::from_secs(3600);
const DATE_FORMAT: &str = "%Y-%m-%d";

#[derive(Debug, thiserror::Error)]
pub enum AzureStorageError {
    #[error("the Azure storage payload could not be serialized")]
    Serialize(#[source] PrivateJsonError),
    #[error("the Azure storage token could not be obtained")]
    Token(#[from] TokenError),
    #[error("the Azure storage request could not be signed")]
    Signing(#[from] SigningError),
    #[error("the Azure storage request could not be sent")]
    Transport(#[from] StorageTransportError),
    #[error("the Azure storage service answered with status {status} for {operation}")]
    Status {
        operation: &'static str,
        status: u16,
    },
    #[error("the injected clock cannot produce a calendar date")]
    Date,
    #[error("the Azure Data Lake client needs AZURE_STORAGE_ACCOUNT_KEY")]
    MissingAccountKey,
}

#[must_use]
pub fn dated_directory(now: Duration) -> Option<String> {
    let seconds = i64::try_from(now.as_secs()).ok()?;
    chrono::DateTime::from_timestamp(seconds, 0)
        .map(|moment| moment.format(DATE_FORMAT).to_string())
}

pub struct AzureStorageDeps {
    pub transport: Arc<dyn StorageTransport>,
    pub clock: Arc<dyn Clock>,
    pub environment: Arc<dyn EnvironmentLookup + Send + Sync>,
    pub ids: Arc<dyn IdSource>,
    pub chain_factory: Arc<dyn ChainProviderFactory>,
    pub shutdown: CancellationToken,
    // the server owns the flush timer's task; without a tracker only explicit flushes run
    pub tasks: Option<TaskTracker>,
}

#[derive(Clone)]
struct CachedClient {
    account: String,
    key: secrecy::SecretString,
    expires_at: Duration,
}

#[derive(Default)]
pub struct UploadState {
    token: TokenState,
    chain: Option<Arc<dyn ChainTokenProvider>>,
    client: Option<CachedClient>,
}

pub struct AzureBlobStorageLogger {
    pub settings: AzureStorageSettings,
    pub deps: AzureStorageDeps,
    pub queue: BatchQueue<StandardLoggingPayload>,
    pub state: Mutex<UploadState>,
    pub last_flush_time: Mutex<Duration>,
    // the account-key client lifecycle, so a flush and a test both see how often the SDK client was rebuilt
    pub created_clients: AtomicUsize,
    pub closed_clients: AtomicUsize,
}

impl AzureBlobStorageLogger {
    #[must_use]
    pub fn new(settings: AzureStorageSettings, deps: AzureStorageDeps) -> Arc<Self> {
        let last_flush_time = deps.clock.now_unix();
        let logger = Arc::new(Self {
            queue: BatchQueue::new(settings.batch),
            settings,
            deps,
            state: Mutex::new(UploadState::default()),
            last_flush_time: Mutex::new(last_flush_time),
            created_clients: AtomicUsize::new(0),
            closed_clients: AtomicUsize::new(0),
        });
        if let Some(tasks) = logger.deps.tasks.clone() {
            let periodic = Arc::clone(&logger);
            let shutdown = logger.deps.shutdown.clone();
            let interval = logger.settings.batch.flush_interval;
            tasks.spawn(async move { periodic.run_flush_loop(interval, shutdown).await });
        }
        logger
    }

    #[must_use]
    pub fn last_flush_time(&self) -> Duration {
        *self
            .last_flush_time
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    // CustomBatchLogger.periodic_flush: sleep the interval, then flush unless the server is shutting down
    async fn run_flush_loop(&self, interval: Duration, shutdown: CancellationToken) {
        loop {
            let _woken = timeout(interval, shutdown.cancelled()).await;
            if shutdown.is_cancelled() {
                break;
            }
            let report = self.flush_queue().await;
            if report.cancelled {
                tracing::warn!(
                    batch = report.batch,
                    "the Azure storage flush was cancelled; its records stay queued"
                );
            }
        }
    }

    // LiteLLM sends one file per queued event and stops at the first failure
    pub async fn async_send_batch(&self) -> usize {
        let batch = self.queue.pending();
        if batch.is_empty() {
            tracing::error!("Azure storage: log_queue does not exist");
            return 0;
        }
        self.send_batch(&batch).await
    }

    async fn send_batch(&self, batch: &[StandardLoggingPayload]) -> usize {
        let mut uploaded = 0_usize;
        for payload in batch {
            if let Err(error) = self.upload_payload(payload).await {
                tracing::error!(%error, "AzureBlobStorageLogger Error sending batch API");
                break;
            }
            uploaded = uploaded.saturating_add(1);
        }
        uploaded
    }

    // a failed flush keeps the failed event and everything after it queued for the next cycle
    pub async fn flush_queue(&self) -> FlushReport {
        let _flush = self.queue.flush_lock.lock().await;
        let batch = self.queue.pending();
        if batch.is_empty() {
            return FlushReport {
                batch: 0,
                delivered: 0,
                requeued: 0,
                dropped_oldest: 0,
                cancelled: false,
            };
        }
        let uploaded = self.send_batch(&batch).await;
        let requeued = batch.len().saturating_sub(uploaded);
        {
            let mut queue = self
                .queue
                .queue
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            for _removed in 0..uploaded {
                let _sent = queue.pop_front();
            }
        }
        if requeued == 0 {
            *self
                .last_flush_time
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = self.deps.clock.now_unix();
        }
        FlushReport {
            batch: batch.len(),
            delivered: uploaded,
            requeued,
            dropped_oldest: 0,
            cancelled: false,
        }
    }
}

#[async_trait]
impl CallbackLogger for AzureBlobStorageLogger {
    fn callback_name(&self) -> &str {
        CALLBACK_NAME
    }

    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        let _enqueued = self.queue.enqueue((*event.payload).clone());
        Ok(())
    }

    async fn async_log_failure_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        let _enqueued = self.queue.enqueue((*event.payload).clone());
        Ok(())
    }
}
