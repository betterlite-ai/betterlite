use std::error::Error as StdError;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use secrecy::SecretString;
use thiserror::Error;

// stands for VertexBase._ensure_access_token: the credential cache turns a deployment's credentials into a bearer token and the project they resolve
#[async_trait]
pub trait VertexTokenPort: Send + Sync {
    async fn access_token(
        &self,
        credentials: Option<&ConfiguredCredentials>,
        project: Option<&str>,
    ) -> Result<(SecretString, String), PortError>;
}

// every call carries the bearer token and its own timeout; a non-2xx status is an HttpReply, not a port failure
// post_json sends Content-Type: application/json, get sends only Authorization, and post_multipart sends
// X-Goog-Upload-Protocol: multipart with the metadata part typed application/json
#[async_trait]
pub trait VertexRagHttp: Send + Sync {
    async fn post_json(&self, request: JsonRequest<'_>) -> Result<HttpReply, PortError>;

    async fn get(&self, request: GetRequest<'_>) -> Result<HttpReply, PortError>;

    async fn post_multipart(&self, request: MultipartRequest<'_>) -> Result<HttpReply, PortError>;
}

pub struct HttpReply {
    pub status: u16,
    pub body: Vec<u8>,
}

pub struct JsonRequest<'request> {
    pub url: &'request str,
    pub token: &'request SecretString,
    pub body: &'request [u8],
    pub timeout: Duration,
}

pub struct GetRequest<'request> {
    pub url: &'request str,
    pub token: &'request SecretString,
    pub timeout: Duration,
}

pub struct MultipartRequest<'request> {
    pub url: &'request str,
    pub token: &'request SecretString,
    pub metadata: &'request [u8],
    pub filename: &'request str,
    pub file: &'request [u8],
    pub file_content_type: &'request str,
    pub timeout: Duration,
}

pub struct VertexIngestionPorts {
    pub token: Arc<dyn VertexTokenPort>,
    pub http: Arc<dyn VertexRagHttp>,
}

#[derive(Debug, Error)]
#[error("{message}")]
pub struct PortError {
    message: String,
    #[source]
    source: Box<dyn StdError + Send + Sync>,
}

impl PortError {
    #[must_use]
    pub fn new<Failure>(failure: Failure) -> Self
    where
        Failure: StdError + Send + Sync + 'static,
    {
        Self {
            message: failure.to_string(),
            source: Box::new(failure),
        }
    }
}
