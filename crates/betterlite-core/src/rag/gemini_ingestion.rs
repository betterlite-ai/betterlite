mod tests;
mod wire;

pub mod port;
pub mod reply;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use http::header::InvalidHeaderValue;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;
use tracing::error;

use self::port::{
    GeminiTransport, TransportBody, TransportError, TransportMethod, TransportRequest,
};
use self::reply::{ReplyError, decode_upload_reply};
use self::wire::{
    API_VERSION, body_text, initiate_body, initiate_headers, initiate_url, json_headers,
    put_headers, store_name,
};
use super::ingestion::IngestionState;
use super::ingestion::config::VectorStoreConfig;
use super::ingestion::store::{RagStore, StoreRequest, StoreResult};

const DEFAULT_INGEST_NAME: &str = "litellm-rag-ingest";
const CREATE_TIMEOUT: Duration = Duration::from_secs(60);
const FINALIZE_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Clone)]
pub struct GeminiDefaults {
    pub api_key: Option<SecretString>,
    pub api_base: String,
}

pub struct GeminiIngestion {
    pub state: IngestionState,
    pub defaults: GeminiDefaults,
    pub transport: Arc<dyn GeminiTransport>,
}

#[derive(Debug, Error)]
pub enum GeminiIngestionError {
    #[error("GEMINI_API_KEY or GOOGLE_API_KEY is required for Gemini File Search")]
    MissingApiKey,
    #[error("Failed to create File Search store: {body}")]
    CreateStoreFailed { body: String },
    #[error("Failed to initiate upload: {body}")]
    InitiateFailed { body: String },
    #[error("No upload URL returned in response headers")]
    NoUploadUrl,
    #[error("Failed to upload file: {body}")]
    UploadFailed { body: String },
    #[error("the vector_store.{field} setting must be {expected}")]
    Setting {
        field: &'static str,
        expected: &'static str,
    },
    #[error("the {name} header value is not a header value")]
    Header {
        name: &'static str,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the File Search store reply could not be read")]
    BadReply {
        #[source]
        source: ReplyError,
    },
    #[error(transparent)]
    Transport(#[from] TransportError),
}

// The ingest pipeline's embedding step: Gemini embeds inside the store upload, so this yields nothing
pub trait EmbedStep {
    fn embed(&self, chunks: &[String]) -> Option<Vec<Vec<f64>>>;
}

impl EmbedStep for GeminiIngestion {
    fn embed(&self, _chunks: &[String]) -> Option<Vec<Vec<f64>>> {
        None
    }
}

#[async_trait]
impl RagStore for GeminiIngestion {
    type Error = GeminiIngestionError;

    async fn store(&self, request: StoreRequest<'_>) -> Result<StoreResult, Self::Error> {
        let config = &self.state.vector_store_config;
        let api_key = resolved_key(config, &self.defaults)?;
        let base = text_field(config, "api_base")?.filter(|value| !value.is_empty());
        let base_url = format!(
            "{}/{API_VERSION}",
            base.unwrap_or_else(|| self.defaults.api_base.clone())
        );
        let configured_id =
            text_field(config, "vector_store_id")?.filter(|value| !value.is_empty());
        let store_id = match configured_id {
            Some(identified) => identified,
            None => self.create_store(&api_key, &base_url).await?,
        };
        let mut file_id = None;
        if let Some((content, filename)) = upload_form(&request, &store_id) {
            let site = UploadSite {
                base_url: &base_url,
                store_id: &store_id,
                filename,
            };
            let upload_url = self
                .initiate(&site, &api_key, content.len(), request.content_type)
                .await?;
            file_id = Some(self.finalize(&upload_url, content).await?);
        }
        Ok((Some(store_id), file_id))
    }
}

struct UploadSite<'site> {
    base_url: &'site str,
    store_id: &'site str,
    filename: &'site str,
}

impl GeminiIngestion {
    pub const SUPPORTS_EXISTING_FILE_ID: bool = IngestionState::SUPPORTS_EXISTING_FILE_ID;

    async fn create_store(
        &self,
        api_key: &str,
        base_url: &str,
    ) -> Result<String, GeminiIngestionError> {
        let name = self
            .state
            .ingest_name
            .as_deref()
            .filter(|value| !value.is_empty())
            .unwrap_or(DEFAULT_INGEST_NAME);
        let mut body = IndexMap::new();
        body.insert("displayName".to_owned(), Json::Text(name.to_owned()));
        let reply = self
            .transport
            .send(TransportRequest {
                method: TransportMethod::Post,
                url: format!("{base_url}/fileSearchStores"),
                headers: json_headers(api_key)?,
                body: TransportBody::Json(Json::Object(body)),
                timeout: CREATE_TIMEOUT,
            })
            .await?;
        if reply.status != 200 {
            let body = body_text(&reply.body);
            error!("Failed to create File Search store: {body}");
            return Err(GeminiIngestionError::CreateStoreFailed { body });
        }
        store_name(&reply.body)
    }

    async fn initiate(
        &self,
        site: &UploadSite<'_>,
        api_key: &str,
        file_size: usize,
        content_type: Option<&str>,
    ) -> Result<String, GeminiIngestionError> {
        let body = initiate_body(
            site.filename,
            self.state.chunking_strategy.as_ref(),
            &self.state.vector_store_config,
        )?;
        let reply = self
            .transport
            .send(TransportRequest {
                method: TransportMethod::Post,
                url: initiate_url(site.base_url, site.store_id),
                headers: initiate_headers(api_key, file_size, content_type)?,
                body: TransportBody::Json(body),
                timeout: CREATE_TIMEOUT,
            })
            .await?;
        if !matches!(reply.status, 200 | 201) {
            let body = body_text(&reply.body);
            error!("Failed to initiate upload: {body}");
            return Err(GeminiIngestionError::InitiateFailed { body });
        }
        reply
            .headers
            .get("x-goog-upload-url")
            .and_then(|value| value.to_str().ok())
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| GeminiIngestionError::NoUploadUrl)
    }

    async fn finalize(
        &self,
        upload_url: &str,
        content: &[u8],
    ) -> Result<String, GeminiIngestionError> {
        let reply = self
            .transport
            .send(TransportRequest {
                method: TransportMethod::Put,
                url: upload_url.to_owned(),
                headers: put_headers(content.len())?,
                body: TransportBody::Bytes(content.to_vec()),
                timeout: FINALIZE_TIMEOUT,
            })
            .await?;
        if !matches!(reply.status, 200 | 201) {
            let body = body_text(&reply.body);
            error!("Failed to upload file: {body}");
            return Err(GeminiIngestionError::UploadFailed { body });
        }
        Ok(decode_upload_reply(&reply.body).file_id)
    }
}

fn upload_form<'request>(
    request: &StoreRequest<'request>,
    store_id: &str,
) -> Option<(&'request [u8], &'request str)> {
    if store_id.is_empty() {
        return None;
    }
    let content = request.file_content.filter(|bytes| !bytes.is_empty())?;
    let filename = request.filename.filter(|name| !name.is_empty())?;
    Some((content, filename))
}

fn resolved_key(
    config: &VectorStoreConfig,
    defaults: &GeminiDefaults,
) -> Result<String, GeminiIngestionError> {
    if let Some(key) = text_field(config, "api_key")?.filter(|value| !value.is_empty()) {
        return Ok(key);
    }
    let default_key = defaults
        .api_key
        .as_ref()
        .map(|key| key.expose_secret().to_owned())
        .filter(|value| !value.is_empty());
    default_key.ok_or_else(|| GeminiIngestionError::MissingApiKey)
}

fn text_field(
    config: &VectorStoreConfig,
    field: &'static str,
) -> Result<Option<String>, GeminiIngestionError> {
    let Some(raw) = config.get(field) else {
        return Ok(None);
    };
    serde_json::from_str(raw.get()).map_err(|_source| GeminiIngestionError::Setting {
        field,
        expected: "a string",
    })
}
