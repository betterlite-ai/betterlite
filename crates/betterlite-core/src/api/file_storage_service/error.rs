use betterlite_config::api::file_azure_storage::SettingsError;
use betterlite_error::proxy_exception::ProxyException;
use thiserror::Error;

use crate::api::file_azure_storage::error::AzureFileStorageError;
use crate::api::file_storage_contracts::storage::StorageError;

#[derive(Debug, Error)]
pub enum StorageBackendError {
    #[error(
        "Unsupported storage backend type: {backend}. Supported types: azure_storage, litellm_db"
    )]
    Unsupported { backend: String },
    #[error("Storage backend litellm_db requires a database-connected proxy")]
    DatabaseRequired,
    #[error("{0}")]
    Settings(#[from] SettingsError),
}

#[derive(Debug, Error)]
pub enum StorageUploadError {
    #[error(transparent)]
    Azure(#[from] AzureFileStorageError),
    #[error(transparent)]
    Database(#[from] StorageError),
}

// both variants are boxed so the enum stays two pointers wide despite the larger upload error
#[derive(Debug, Error)]
pub enum StorageServiceError {
    #[error(transparent)]
    Proxy(Box<ProxyException>),
    #[error(transparent)]
    Upload(Box<StorageUploadError>),
}

impl From<StorageUploadError> for StorageServiceError {
    fn from(error: StorageUploadError) -> Self {
        Self::Upload(Box::new(error))
    }
}
