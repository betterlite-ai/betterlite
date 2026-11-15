use std::error::Error as StdError;

use async_trait::async_trait;
use betterlite_db::auxiliary_repository::file_content::ManagedFileContentRepository;
use betterlite_db::auxiliary_repository::repository::TableError;
use thiserror::Error;

pub const LITELLM_DB_BACKEND_NAME: &str = "litellm_db";
pub const LITELLM_DB_STORAGE_URL_PREFIX: &str = "litellm_db://";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FileNamingStrategy {
    #[default]
    Uuid,
    Timestamp,
    OriginalFilename,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UploadOptions<'text> {
    pub path_prefix: Option<&'text str>,
    pub file_naming_strategy: FileNamingStrategy,
}

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("Not a litellm_db storage url: {url}")]
    NotLiteLlmDbUrl { url: String },
    #[error("No stored file content for {url}")]
    NoStoredContent { url: String },
    #[error(transparent)]
    Repository(#[from] TableError),
}

#[async_trait]
pub trait FileStorageBackend: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    async fn upload_file(
        &self,
        file_content: &[u8],
        filename: &str,
        content_type: &str,
        options: UploadOptions<'_>,
    ) -> Result<String, Self::Error>;

    async fn download_file(&self, storage_url: &str) -> Result<Vec<u8>, Self::Error>;

    async fn delete_file(&self, _storage_url: &str) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[async_trait]
pub trait FileContentRepository: Send + Sync {
    async fn store_content(&self, content: &[u8]) -> Result<String, TableError>;

    async fn load_content(&self, row_id: &str) -> Result<Option<Vec<u8>>, TableError>;

    async fn delete_content(&self, row_id: &str) -> Result<(), TableError>;
}

#[async_trait]
impl FileContentRepository for ManagedFileContentRepository<'_> {
    async fn store_content(&self, content: &[u8]) -> Result<String, TableError> {
        self.store(content).await
    }

    async fn load_content(&self, row_id: &str) -> Result<Option<Vec<u8>>, TableError> {
        self.load(row_id).await
    }

    async fn delete_content(&self, row_id: &str) -> Result<(), TableError> {
        self.delete(row_id).await
    }
}

pub fn storage_url_to_row_id(storage_url: &str) -> Result<&str, StorageError> {
    let Some(row_id) = storage_url.strip_prefix(LITELLM_DB_STORAGE_URL_PREFIX) else {
        return Err(StorageError::NotLiteLlmDbUrl {
            url: storage_url.to_owned(),
        });
    };
    Ok(row_id)
}

pub struct LiteLlmDbStorageBackend<R> {
    pub contents: R,
}

#[async_trait]
impl<R: FileContentRepository> FileStorageBackend for LiteLlmDbStorageBackend<R> {
    type Error = StorageError;

    async fn upload_file(
        &self,
        file_content: &[u8],
        _filename: &str,
        _content_type: &str,
        _options: UploadOptions<'_>,
    ) -> Result<String, StorageError> {
        let row_id = self.contents.store_content(file_content).await?;
        Ok(format!("{LITELLM_DB_STORAGE_URL_PREFIX}{row_id}"))
    }

    async fn download_file(&self, storage_url: &str) -> Result<Vec<u8>, StorageError> {
        let row_id = storage_url_to_row_id(storage_url)?;
        let content = self.contents.load_content(row_id).await?;
        content.ok_or_else(|| StorageError::NoStoredContent {
            url: storage_url.to_owned(),
        })
    }

    async fn delete_file(&self, storage_url: &str) -> Result<(), StorageError> {
        let row_id = storage_url_to_row_id(storage_url)?;
        self.contents.delete_content(row_id).await?;
        Ok(())
    }
}
