use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::loader::EnvironmentLookup;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::batch::logger::BatchQueue;
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

mod account_key;
mod upload;

pub const CALLBACK_NAME: &str = "azure_storage";
// LiteLLM's _DEFAULT_TTL_FOR_HTTPX_CLIENTS, the lifetime of a cached DataLakeServiceClient
pub const SERVICE_CLIENT_TTL: Duration = Duration::from_secs(3600);
// raise_for_status turns any of these into an upload error
pub const FIRST_ERROR_STATUS: u16 = 400;
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
