use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use secrecy::SecretString;
use tokio::runtime::Handle;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use crate::batch::logger::{BatchQueue, FlushReport};
use crate::fields::payload::StandardLoggingPayload;
use crate::private_json::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, safe_dumps};

use super::account_key::{ensure_directory, service_client, write_file};
use super::{
    AzureBlobStorageLogger, AzureStorageDeps, AzureStorageError, FIRST_ERROR_STATUS, UploadState,
    dated_directory,
};
use crate::azure_storage_logging::filename::{adls_safe_file_name, storage_url};
use crate::azure_storage_logging::http_ops::StorageAuthorization;
use crate::azure_storage_logging::settings::AzureStorageSettings;
use crate::azure_storage_logging::token::{
    ChainTokenProvider, TokenError, authority_host, client_secret_request, credentials,
    parse_token_response,
};
use crate::azure_storage_logging::transport::{StorageRequest, StorageResponse};

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

    pub async fn upload_payload(
        &self,
        payload: &StandardLoggingPayload,
    ) -> Result<(), AzureStorageError> {
        if self.settings.account_key.is_some() {
            self.upload_with_account_key(payload).await
        } else {
            self.upload_with_token(payload).await
        }
    }

    async fn upload_with_token(
        &self,
        payload: &StandardLoggingPayload,
    ) -> Result<(), AzureStorageError> {
        let token = self.set_valid_azure_ad_token().await?;
        let dumped = safe_dumps(payload, DEFAULT_MAX_RECURSE_DEPTH, None)
            .map_err(AzureStorageError::Serialize)?;
        let file_name = adls_safe_file_name(Some(payload.id.as_str()), self.deps.ids.as_ref());
        let base_url = storage_url(
            &self.settings.dfs_endpoint,
            &[self.settings.file_system.as_str(), file_name.as_str()],
        );
        let auth = StorageAuthorization::Bearer(&token);
        write_file(self, &base_url, &auth, format!("{dumped}\n").as_bytes()).await
    }

    async fn upload_with_account_key(
        &self,
        payload: &StandardLoggingPayload,
    ) -> Result<(), AzureStorageError> {
        let now = self.deps.clock.now_unix();
        let client = service_client(self, now)?;
        let auth = StorageAuthorization::SharedKey {
            account: &client.account,
            key: &client.key,
        };
        let directory = dated_directory(now).ok_or(AzureStorageError::Date)?;
        let segments = [self.settings.file_system.as_str(), directory.as_str()];
        let directory_url = storage_url(&self.settings.dfs_endpoint, &segments);
        let exists_url = storage_url(&self.settings.blob_endpoint, &segments);
        ensure_directory(self, &exists_url, &directory_url, &auth).await?;
        let file_name = adls_safe_file_name(Some(payload.id.as_str()), self.deps.ids.as_ref());
        let content = safe_dumps(payload, DEFAULT_MAX_RECURSE_DEPTH, None)
            .map_err(AzureStorageError::Serialize)?;
        let base_url = storage_url(&directory_url, &[file_name.as_str()]);
        write_file(self, &base_url, &auth, content.as_bytes()).await
    }

    pub(super) async fn execute(
        &self,
        request: StorageRequest,
        operation: &'static str,
    ) -> Result<StorageResponse, AzureStorageError> {
        let response = self.deps.transport.send(request).await?;
        if response.status >= FIRST_ERROR_STATUS {
            return Err(AzureStorageError::Status {
                operation,
                status: response.status,
            });
        }
        Ok(response)
    }

    // LiteLLM caches the credential-chain token provider in its factory, so construction happens once
    fn chain_provider(&self) -> Result<Arc<dyn ChainTokenProvider>, TokenError> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(provider) = &state.chain {
            return Ok(Arc::clone(provider));
        }
        let provider = self.deps.chain_factory.provider()?;
        state.chain = Some(Arc::clone(&provider));
        drop(state);
        Ok(provider)
    }

    pub async fn set_valid_azure_ad_token(&self) -> Result<SecretString, TokenError> {
        if self.settings.tenant_id.is_none()
            && self.settings.client_id.is_none()
            && self.settings.client_secret.is_none()
        {
            return self.chain_token().await;
        }
        let now = self.deps.clock.now_unix();
        let cached = {
            let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            if state.token.is_expired(now) {
                None
            } else {
                state.token.token.clone()
            }
        };
        if let Some(token) = cached {
            return Ok(token);
        }
        let credentials = credentials(&self.settings, self.deps.environment.as_ref())?;
        let authority = authority_host(self.deps.environment.as_ref());
        let request = client_secret_request(&credentials, &authority)?;
        let response = self.deps.transport.send(request).await?;
        let token = parse_token_response(&response)?;
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .token
            .store(token.clone(), now);
        Ok(token)
    }

    // the chain walk probes IMDS and may run a CLI, so it runs off the async worker
    async fn chain_token(&self) -> Result<SecretString, TokenError> {
        let provider = self.chain_provider()?;
        let token = Handle::current()
            .spawn_blocking(move || provider.token())
            .await
            .map_err(TokenError::Worker)??;
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .token
            .token = Some(token.clone());
        Ok(token)
    }
}
