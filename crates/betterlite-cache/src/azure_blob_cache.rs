use std::sync::Arc;
use std::time::Duration;

use azure_core::Error as AzureError;
use azure_core::credentials::TokenCredential;
use azure_core::error::ErrorKind;
use azure_core::http::{RequestContent, StatusCode, Transport};
use azure_storage_blob::BlobContainerClient;
use azure_storage_blob::models::{BlobClientUploadOptions, StorageErrorCode};
use betterlite_config::sections::azure_blob_cache::AzureBlobSettings;
use tokio::runtime::Handle;
use tokio::task::JoinSet;
use tokio::time::error::Elapsed;
use tokio::time::timeout;

pub mod body;
mod client;
use crate::s3_cache::entry::CachedEntry;
use body::decode_body;

#[derive(Clone)]
pub struct AzureBlobCache {
    container_client: Arc<BlobContainerClient>,
    operation_timeout: Duration,
}

impl AzureBlobCache {
    pub async fn new(
        settings: &AzureBlobSettings,
        credential: Arc<dyn TokenCredential>,
        transport: Transport,
        operation_timeout: Duration,
    ) -> Result<Self, CacheError> {
        let container_client = client::build(settings, credential, transport)?;
        let create = container_client.create(None);
        match timeout(operation_timeout, create).await {
            Ok(Ok(_)) => {}
            Ok(Err(source)) if is_container_already_exists(&source) => {}
            Ok(Err(source)) => return Err(CacheError::ContainerCreate { source }),
            Err(source) => return Err(CacheError::ContainerCreateTimeout { source }),
        }
        Ok(Self {
            container_client: Arc::new(container_client),
            operation_timeout,
        })
    }

    pub fn set_cache(&self, runtime: &Handle, key: &str, value: &CachedEntry) {
        if Handle::try_current().is_ok() {
            tracing::warn!(
                error = "sync cache called inside async runtime",
                "Azure Blob cache write failed"
            );
            return;
        }
        runtime.block_on(self.put(key, value, false));
    }

    pub async fn async_set_cache(&self, key: &str, value: &CachedEntry) {
        self.put(key, value, true).await;
    }

    async fn put(&self, key: &str, value: &CachedEntry, overwrite: bool) {
        let serialized = match serde_json::to_vec(value) {
            Ok(body) => body,
            Err(source) => {
                tracing::warn!(error = %source, "Azure Blob cache serialization failed");
                return;
            }
        };
        let blob_client = self.container_client.blob_client(key);
        let content = RequestContent::from(serialized);
        let options = if overwrite {
            None
        } else {
            Some(BlobClientUploadOptions::default().if_not_exists())
        };
        let upload = blob_client.upload(content, options);
        match timeout(self.operation_timeout, upload).await {
            Ok(Ok(_)) => {}
            Ok(Err(source)) => tracing::warn!(error = %source, "Azure Blob cache write failed"),
            Err(source) => tracing::warn!(error = %source, "Azure Blob cache write timed out"),
        }
    }

    pub fn get_cache(&self, runtime: &Handle, key: &str) -> Option<CachedEntry> {
        if Handle::try_current().is_ok() {
            tracing::error!(
                error = "sync cache called inside async runtime",
                "Azure Blob cache read failed"
            );
            return None;
        }
        runtime.block_on(self.async_get_cache(key))
    }

    pub async fn async_get_cache(&self, key: &str) -> Option<CachedEntry> {
        let fetch = async {
            let blob_client = self.container_client.blob_client(key);
            let download = match blob_client.download(None).await {
                Ok(download) => download,
                Err(source) if is_blob_not_found(&source) => {
                    tracing::debug!(error = %source, "Azure Blob cache miss");
                    return None;
                }
                Err(source) => {
                    tracing::error!(error = %source, "Azure Blob cache read failed");
                    return None;
                }
            };
            let bytes = match download.body.collect().await {
                Ok(bytes) => bytes,
                Err(source) => {
                    tracing::error!(error = %source, "Azure Blob cache body failed");
                    return None;
                }
            };
            match decode_body(&bytes) {
                Ok(value) => Some(value),
                Err(source) => {
                    tracing::error!(error = %source, "Azure Blob cache decode failed");
                    None
                }
            }
        };
        match timeout(self.operation_timeout, fetch).await {
            Ok(result) => result,
            Err(source) => {
                tracing::error!(error = %source, "Azure Blob cache read timed out");
                None
            }
        }
    }

    pub async fn async_set_cache_pipeline(&self, entries: Vec<(String, CachedEntry)>) {
        let mut tasks = JoinSet::new();
        for (key, value) in entries {
            if tasks.len() >= 16
                && let Some(Err(source)) = tasks.join_next().await
            {
                tracing::warn!(error = %source, "Azure Blob cache pipeline failed");
            }
            let cache = self.clone();
            tasks.spawn(async move {
                cache.async_set_cache(&key, &value).await;
            });
        }
        while let Some(result) = tasks.join_next().await {
            if let Err(source) = result {
                tracing::warn!(error = %source, "Azure Blob cache pipeline failed");
            }
        }
    }
}

fn is_container_already_exists(error: &AzureError) -> bool {
    is_error(
        error,
        StatusCode::Conflict,
        &StorageErrorCode::ContainerAlreadyExists,
    )
}

fn is_blob_not_found(error: &AzureError) -> bool {
    is_error(error, StatusCode::NotFound, &StorageErrorCode::BlobNotFound)
}

fn is_error(error: &AzureError, status: StatusCode, storage_code: &StorageErrorCode) -> bool {
    error.http_status() == Some(status)
        && matches!(
            error.kind(),
            ErrorKind::HttpResponse {
                error_code: Some(code),
                ..
            } if code == storage_code.as_ref()
        )
}

#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("Azure Blob client configuration is invalid")]
    InvalidUrl {
        #[source]
        source: AzureError,
    },
    #[error("Azure Blob container client could not be built")]
    Client {
        #[source]
        source: AzureError,
    },
    #[error("Azure Blob container creation failed")]
    ContainerCreate {
        #[source]
        source: AzureError,
    },
    #[error("Azure Blob container creation timed out")]
    ContainerCreateTimeout {
        #[source]
        source: Elapsed,
    },
}

pub trait Disconnect {
    fn disconnect(&self) -> impl Future<Output = ()> + Send;
}

impl Disconnect for AzureBlobCache {
    async fn disconnect(&self) {}
}

mod tests;
