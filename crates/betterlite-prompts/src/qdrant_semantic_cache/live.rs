use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use http::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

use super::error::QdrantSemanticError;
use super::ports::{QdrantReply, QdrantRequest, QdrantTransport};

// LiteLLM's HTTPHandler sends with its 600 second completion timeout
const QDRANT_DEADLINE: Duration = Duration::from_secs(600);
// Qdrant reads the key from exactly this header name, spelled in lower case like httpx sends it
const API_KEY_HEADER: &str = "api-key";

pub struct LiveQdrantTransport {
    pub client: ProxiedClient,
    pub api_base: String,
    pub headers: HeaderMap,
    pub deadline: Duration,
}

impl LiveQdrantTransport {
    pub fn new(
        client: ProxiedClient,
        api_base: String,
        api_key: Option<SecretString>,
    ) -> Result<Self, QdrantSemanticError> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if let Some(key) = api_key {
            let mut value = HeaderValue::from_str(key.expose_secret())
                .map_err(QdrantSemanticError::InvalidApiKey)?;
            value.set_sensitive(true);
            headers.insert(HeaderName::from_static(API_KEY_HEADER), value);
        }
        Ok(Self {
            client,
            api_base,
            headers,
            deadline: QDRANT_DEADLINE,
        })
    }

    pub fn build_request(
        &self,
        request: QdrantRequest,
    ) -> Result<OutboundRequest, QdrantSemanticError> {
        let url = Url::parse(&format!("{}{}", self.api_base, request.path)).map_err(|source| {
            QdrantSemanticError::InvalidUrl {
                path: request.path.clone(),
                source,
            }
        })?;
        Ok(OutboundRequest {
            method: request.method,
            url,
            query: request.query,
            headers: self.headers.clone(),
            body: request.body.map_or(RequestBody::Empty, RequestBody::Json),
            deadline: self.deadline,
        })
    }
}

#[async_trait]
impl QdrantTransport for LiveQdrantTransport {
    async fn send(&self, request: QdrantRequest) -> Result<QdrantReply, QdrantSemanticError> {
        let path = request.path.clone();
        let built = self.build_request(request)?;
        let reply = self
            .client
            .send_buffered(built)
            .await
            .map_err(|source| QdrantSemanticError::Transport { path, source })?;
        Ok(QdrantReply {
            status: reply.status,
            body: reply.body,
        })
    }
}
