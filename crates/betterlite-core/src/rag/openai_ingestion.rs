use std::error::Error as StdError;
use std::sync::Arc;

use async_trait::async_trait;
use secrecy::SecretString;
use serde::de::DeserializeOwned;
use serde_json::value::RawValue;
use thiserror::Error;

use crate::rag::ingestion::IngestionState;
use crate::rag::ingestion::config::VectorStoreConfig;
use crate::rag::ingestion::store::{RagStore, StoreRequest, StoreResult};
use crate::rag::ingestion::upload::DEFAULT_CONTENT_TYPE;

mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExpiresAfter {
    pub anchor: &'static str,
    pub days: i64,
}

#[derive(Debug)]
pub struct CreateStore<'request> {
    pub name: &'request str,
    pub expires_after: Option<ExpiresAfter>,
    pub api_key: Option<&'request SecretString>,
    pub api_base: Option<&'request str>,
}

#[derive(Debug)]
pub struct AttachFile<'request> {
    pub vector_store_id: &'request str,
    pub file_id: &'request str,
    pub chunking_strategy: &'request RawValue,
    pub api_key: Option<&'request SecretString>,
    pub api_base: Option<&'request str>,
}

#[derive(Debug)]
pub struct UploadFile<'request> {
    pub filename: &'request str,
    pub content: &'request [u8],
    pub content_type: &'request str,
    pub purpose: &'request str,
    pub api_key: Option<&'request SecretString>,
    pub api_base: Option<&'request str>,
}

// the ports take the SDK's own failure as a boxed source, so an adapter keeps its typed error while B3460 keeps its message
#[derive(Debug, Error)]
#[error("{message}")]
pub struct PortError {
    pub message: String,
    #[source]
    pub source: Box<dyn StdError + Send + Sync>,
}

#[async_trait]
pub trait VectorStorePort: Send + Sync {
    async fn create_store(&self, request: CreateStore<'_>) -> Result<Option<String>, PortError>;
}

#[async_trait]
pub trait VectorFilePort: Send + Sync {
    async fn attach(&self, request: AttachFile<'_>) -> Result<(), PortError>;
}

#[async_trait]
pub trait FileUploadPort: Send + Sync {
    async fn upload(&self, request: UploadFile<'_>) -> Result<String, PortError>;
}

#[derive(Debug, Error)]
pub enum OpenAiIngestionError {
    #[error("vector_store_id is required when ingesting an existing file_id")]
    MissingVectorStoreId,
    #[error("`{field}` must be {expected}")]
    FieldType {
        field: &'static str,
        expected: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Port(#[from] PortError),
}

pub struct OpenAiIngestion {
    pub state: IngestionState,
    pub vector_store: Arc<dyn VectorStorePort>,
    pub vector_file: Arc<dyn VectorFilePort>,
    pub file_upload: Arc<dyn FileUploadPort>,
}

impl OpenAiIngestion {
    pub const SUPPORTS_EXISTING_FILE_ID: bool = true;

    #[must_use]
    // ast-grep-ignore: no-one-line-functions - B3460's pipeline calls this fixed provider step before store
    pub const fn embed(&self, _chunks: &[String]) -> Option<Vec<Vec<f64>>> {
        None
    }
}

#[async_trait]
impl RagStore for OpenAiIngestion {
    type Error = OpenAiIngestionError;

    async fn store(&self, request: StoreRequest<'_>) -> Result<StoreResult, Self::Error> {
        let config = &self.state.vector_store_config;
        let mut vector_store_id =
            field::<String>(config, "vector_store_id", "a string")?.filter(|id| !id.is_empty());
        let ttl_days =
            field::<i64>(config, "ttl_days", "a whole number")?.filter(|days| *days != 0);
        let api_key = field::<String>(config, "api_key", "a string")?.map(SecretString::from);
        let api_base = field::<String>(config, "api_base", "a string")?;
        let existing_file_id = request.existing_file_id.filter(|id| !id.is_empty());

        if existing_file_id.is_some() && vector_store_id.is_none() {
            return Err(OpenAiIngestionError::MissingVectorStoreId);
        }

        if vector_store_id.is_none() {
            vector_store_id = self
                .vector_store
                .create_store(CreateStore {
                    name: self
                        .state
                        .ingest_name
                        .as_deref()
                        .filter(|name| !name.is_empty())
                        .unwrap_or("litellm-rag-ingest"),
                    expires_after: ttl_days.map(|days| ExpiresAfter {
                        anchor: "last_active_at",
                        days,
                    }),
                    api_key: api_key.as_ref(),
                    api_base: api_base.as_deref(),
                })
                .await?;
        }

        let Some(store_id) = vector_store_id
            .as_deref()
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
        else {
            return Ok((vector_store_id, None));
        };

        if let Some(file_id) = existing_file_id {
            self.vector_file
                .attach(AttachFile {
                    vector_store_id: &store_id,
                    file_id,
                    chunking_strategy: &self.state.chunking_strategy,
                    api_key: api_key.as_ref(),
                    api_base: api_base.as_deref(),
                })
                .await?;
            return Ok((Some(store_id), Some(file_id.to_owned())));
        }

        let Some(content) = request.file_content else {
            return Ok((Some(store_id), None));
        };
        let Some(filename) = request.filename.filter(|name| !name.is_empty()) else {
            return Ok((Some(store_id), None));
        };
        let uploaded = self
            .file_upload
            .upload(UploadFile {
                filename,
                content,
                content_type: request
                    .content_type
                    .filter(|value| !value.is_empty())
                    .unwrap_or(DEFAULT_CONTENT_TYPE),
                purpose: "assistants",
                api_key: api_key.as_ref(),
                api_base: api_base.as_deref(),
            })
            .await?;
        self.vector_file
            .attach(AttachFile {
                vector_store_id: &store_id,
                file_id: &uploaded,
                chunking_strategy: &self.state.chunking_strategy,
                api_key: api_key.as_ref(),
                api_base: api_base.as_deref(),
            })
            .await?;
        Ok((Some(store_id), Some(uploaded)))
    }
}

// absent and null both read as None, and a value of another type names its field instead of reaching a port
fn field<T>(
    config: &VectorStoreConfig,
    name: &'static str,
    expected: &'static str,
) -> Result<Option<T>, OpenAiIngestionError>
where
    T: DeserializeOwned,
{
    let Some(raw) = config.get(name) else {
        return Ok(None);
    };
    serde_json::from_str(raw.get()).map_err(|source| OpenAiIngestionError::FieldType {
        field: name,
        expected,
        source,
    })
}
