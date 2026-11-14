use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::Method;
use http::header::{HeaderMap, HeaderName, HeaderValue, InvalidHeaderName, InvalidHeaderValue};
use url::{ParseError, Url};

// LiteLLM's cached logging client keeps its fallback total, so a dead endpoint cannot wedge a flush forever
pub const STORAGE_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Debug)]
pub struct StorageRequest {
    pub method: Method,
    pub url: String,
    pub query: Vec<(String, String)>,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub deadline: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum StorageTransportError {
    #[error("the Azure storage request URL {url} is not valid")]
    Url {
        url: String,
        #[source]
        source: ParseError,
    },
    #[error("an Azure storage request header name is not valid")]
    HeaderName(#[source] InvalidHeaderName),
    #[error("an Azure storage request header value is not valid")]
    HeaderValue(#[source] InvalidHeaderValue),
    #[error("the Azure storage request could not be completed")]
    Request(#[source] TransportError),
}

#[async_trait]
pub trait StorageTransport: Send + Sync {
    async fn send(&self, request: StorageRequest)
    -> Result<StorageResponse, StorageTransportError>;
}

// the logging client B173's pool hands out, so storage calls share its TLS, proxy and limits
#[derive(Clone, Debug)]
pub struct PooledTransport {
    pub client: ProxiedClient,
}

#[async_trait]
impl StorageTransport for PooledTransport {
    async fn send(
        &self,
        request: StorageRequest,
    ) -> Result<StorageResponse, StorageTransportError> {
        let url = Url::parse(&request.url).map_err(|source| StorageTransportError::Url {
            url: request.url.clone(),
            source,
        })?;
        let mut headers = HeaderMap::new();
        for (name, value) in &request.headers {
            let header_name = HeaderName::from_bytes(name.as_bytes())
                .map_err(StorageTransportError::HeaderName)?;
            let header_value =
                HeaderValue::from_str(value).map_err(StorageTransportError::HeaderValue)?;
            headers.insert(header_name, header_value);
        }
        let body = if request.body.is_empty() {
            RequestBody::Empty
        } else {
            RequestBody::Bytes {
                content_type: HeaderValue::from_static("application/octet-stream"),
                data: request.body,
            }
        };
        let outbound = OutboundRequest {
            method: request.method,
            url,
            query: request.query,
            headers,
            body,
            deadline: request.deadline,
        };
        let response = self
            .client
            .send_buffered(outbound)
            .await
            .map_err(StorageTransportError::Request)?;
        Ok(StorageResponse {
            status: response.status,
            body: response.body,
        })
    }
}
