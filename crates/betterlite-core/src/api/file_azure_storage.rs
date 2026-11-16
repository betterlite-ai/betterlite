mod download;
pub mod error;
pub mod naming;
mod request;
mod tests;
mod upload;

use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::api::file_azure_storage::Settings;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_logging::azure_storage_logging::filename::IdSource;
use betterlite_logging::azure_storage_logging::logger::{
    AzureBlobStorageLogger, AzureStorageDeps, CALLBACK_NAME,
};
use betterlite_logging::azure_storage_logging::settings::AzureStorageSettings;
use betterlite_logging::azure_storage_logging::token::ChainProviderFactory;
use betterlite_logging::azure_storage_logging::transport::StorageTransport;
use betterlite_logging::batch::logger::BatchSettings;
use betterlite_logging::callback_logger::CallbackLogger;
use betterlite_logging::datadog_logging::clock::Clock;
use tokio_util::sync::CancellationToken;

use crate::api::file_storage_contracts::storage::{FileStorageBackend, UploadOptions};

pub struct AzureFileStorageDeps {
    pub transport: Arc<dyn StorageTransport>,
    pub clock: Arc<dyn Clock>,
    pub environment: Arc<dyn EnvironmentLookup + Send + Sync>,
    pub ids: Arc<dyn IdSource>,
    pub chain_factory: Arc<dyn ChainProviderFactory>,
}

pub struct AzureBlobStorageBackend {
    settings: Settings,
    logger: Arc<AzureBlobStorageLogger>,
}

impl AzureBlobStorageBackend {
    #[must_use]
    pub fn new(settings: Settings, deps: AzureFileStorageDeps) -> Arc<Self> {
        let logger = AzureBlobStorageLogger::new(
            logger_settings(&settings),
            AzureStorageDeps {
                transport: Arc::clone(&deps.transport),
                clock: Arc::clone(&deps.clock),
                environment: Arc::clone(&deps.environment),
                ids: Arc::clone(&deps.ids),
                chain_factory: Arc::clone(&deps.chain_factory),
                // the file backend has no flush timer and never queues a record
                shutdown: CancellationToken::new(),
                tasks: None,
            },
        );
        Arc::new(Self { settings, logger })
    }
}

fn logger_settings(settings: &Settings) -> AzureStorageSettings {
    AzureStorageSettings {
        account_name: settings.account_name.clone(),
        file_system: settings.file_system.clone(),
        endpoint_suffix: settings.endpoint_suffix.clone(),
        dfs_endpoint: settings.dfs_endpoint.clone(),
        blob_endpoint: settings.blob_endpoint.clone(),
        tenant_id: settings.tenant_id.clone(),
        client_id: settings.client_id.clone(),
        client_secret: settings.client_secret.clone(),
        account_key: settings.account_key.clone(),
        batch: BatchSettings::default(),
    }
}

#[async_trait]
impl FileStorageBackend for AzureBlobStorageBackend {
    type Error = error::AzureFileStorageError;

    async fn upload_file(
        &self,
        file_content: &[u8],
        filename: &str,
        _content_type: &str,
        options: UploadOptions<'_>,
    ) -> Result<String, Self::Error> {
        match upload::upload_file(self, file_content, filename, options).await {
            Ok(url) => {
                tracing::debug!("Successfully uploaded file to Azure Blob Storage: {url}");
                Ok(url)
            }
            Err(error) => {
                tracing::error!("Error uploading file to Azure Blob Storage: {error}");
                Err(error)
            }
        }
    }

    async fn download_file(&self, storage_url: &str) -> Result<Vec<u8>, Self::Error> {
        match download::download_file(self, storage_url).await {
            Ok(content) => Ok(content),
            Err(error) => {
                tracing::error!("Error downloading file from Azure Blob Storage: {error}");
                Err(error)
            }
        }
    }
}

// the parent logger's success and failure handlers are overridden to do nothing for file storage
impl CallbackLogger for AzureBlobStorageBackend {
    fn callback_name(&self) -> &str {
        CALLBACK_NAME
    }
}
