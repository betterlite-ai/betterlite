use super::error::ApiError;
use async_trait::async_trait;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::header::{HeaderName, HeaderValue};
use http::{HeaderMap, Method};
use indexmap::IndexMap;
use std::time::Duration;
use url::Url;
#[derive(Clone)]
pub struct PostRequest {
    pub endpoint: String,
    pub headers: IndexMap<String, String>,
    pub body: String,
    pub timeout: Duration,
}
#[async_trait]
pub trait PostTransport: Send + Sync {
    async fn post(&self, request: PostRequest) -> Result<u16, ApiError>;
}

// the client a logging callback gets from B173's pool, so posts share its TLS, proxy and limits
#[derive(Clone, Debug)]
pub struct PooledPostTransport {
    pub client: ProxiedClient,
}
#[async_trait]
impl PostTransport for PooledPostTransport {
    async fn post(&self, request: PostRequest) -> Result<u16, ApiError> {
        let url = Url::parse(&request.endpoint).map_err(ApiError::Url)?;
        let mut headers = HeaderMap::new();
        for (name, value) in &request.headers {
            let header_name =
                HeaderName::from_bytes(name.as_bytes()).map_err(|source| ApiError::HeaderName {
                    name: name.clone(),
                    source,
                })?;
            let header_value =
                HeaderValue::try_from(value.as_str()).map_err(|source| ApiError::HeaderValue {
                    name: name.clone(),
                    source,
                })?;
            headers.append(header_name, header_value);
        }
        let outbound = OutboundRequest {
            method: Method::POST,
            url,
            query: Vec::new(),
            headers,
            body: RequestBody::Json(request.body.into_bytes()),
            deadline: request.timeout,
        };
        let response = match self.client.send_buffered(outbound).await {
            Ok(response) => response,
            Err(TransportError::Deadline(expired) | TransportError::ConnectTimeout(expired)) => {
                tracing::debug!(%expired, "the generic API request timed out");
                return Err(ApiError::Timeout);
            }
            Err(other) => return Err(ApiError::Transport(other)),
        };
        Ok(response.status)
    }
}
