use std::sync::Arc;

use betterlite_config::api::file_azure_storage::Settings;

use crate::api::file_azure_storage::{AzureBlobStorageBackend, AzureFileStorageDeps};
use crate::api::file_storage_contracts::storage::{
    FileContentRepository, FileNamingStrategy, FileStorageBackend as _, LITELLM_DB_BACKEND_NAME,
    LiteLlmDbStorageBackend, UploadOptions,
};

use super::error::{StorageBackendError, StorageUploadError};

pub const AZURE_STORAGE_BACKEND_NAME: &str = "azure_storage";

pub struct StorageBackendDeps<R> {
    pub azure: AzureFileStorageDeps,
    // production reads the callback's environment; a harness pins the endpoints its mock captured
    pub azure_settings: Option<Settings>,
    pub database: Option<R>,
}

pub enum StorageBackend<R> {
    Azure(Arc<AzureBlobStorageBackend>),
    LiteLlmDb(Box<LiteLlmDbStorageBackend<R>>),
}

pub fn get_storage_backend<R>(
    backend_type: &str,
    deps: StorageBackendDeps<R>,
) -> Result<StorageBackend<R>, StorageBackendError> {
    tracing::debug!("Creating storage backend: type={backend_type}");
    match backend_type {
        AZURE_STORAGE_BACKEND_NAME => {
            let settings = match deps.azure_settings {
                Some(settings) => settings,
                None => Settings::from_environment(deps.azure.environment.as_ref())?,
            };
            let backend = AzureBlobStorageBackend::new(settings, deps.azure);
            Ok(StorageBackend::Azure(backend))
        }
        LITELLM_DB_BACKEND_NAME => match deps.database {
            Some(contents) => Ok(StorageBackend::LiteLlmDb(Box::new(
                LiteLlmDbStorageBackend { contents },
            ))),
            None => Err(StorageBackendError::DatabaseRequired),
        },
        other => Err(StorageBackendError::Unsupported {
            backend: other.to_owned(),
        }),
    }
}

impl<R: FileContentRepository> StorageBackend<R> {
    // LiteLLM asks for the uuid naming strategy and an empty path prefix, which keeps the object name
    pub(crate) async fn upload(
        &self,
        content: &[u8],
        filename: &str,
        content_type: &str,
    ) -> Result<String, StorageUploadError> {
        let options = UploadOptions {
            path_prefix: None,
            file_naming_strategy: FileNamingStrategy::Uuid,
        };
        match self {
            Self::Azure(backend) => Ok(backend
                .upload_file(content, filename, content_type, options)
                .await?),
            Self::LiteLlmDb(backend) => Ok(backend
                .upload_file(content, filename, content_type, options)
                .await?),
        }
    }
}
