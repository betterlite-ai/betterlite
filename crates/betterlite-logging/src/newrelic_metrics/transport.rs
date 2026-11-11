use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::Method;
use http::header::{HeaderMap, HeaderValue};
use url::{ParseError, Url};

// LiteLLM's cached logging client: COMPLETION_HTTP_FALLBACK_SECONDS with a 5-second connect timeout
pub const METRIC_API_TIMEOUT: Duration = Duration::from_secs(600);

pub struct NewRelicPost {
    pub url: &'static str,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum NewRelicSendError {
    #[error("the Metric API endpoint is not a valid URL")]
    Url(#[source] ParseError),
    #[error("the Metric API request could not be completed")]
    Request(#[source] TransportError),
}

#[async_trait]
pub trait NewRelicTransport: Send + Sync {
    async fn post(&self, request: NewRelicPost) -> Result<u16, NewRelicSendError>;
}

pub struct PooledNewRelicTransport {
    pub client: ProxiedClient,
    pub deadline: Duration,
}

#[async_trait]
impl NewRelicTransport for PooledNewRelicTransport {
    async fn post(&self, request: NewRelicPost) -> Result<u16, NewRelicSendError> {
        let url = Url::parse(request.url).map_err(NewRelicSendError::Url)?;
        let outbound = OutboundRequest {
            method: Method::POST,
            url,
            query: Vec::new(),
            headers: request.headers,
            body: RequestBody::Bytes {
                content_type: HeaderValue::from_static("application/json"),
                data: request.body,
            },
            deadline: self.deadline,
        };
        let response = self
            .client
            .send_buffered(outbound)
            .await
            .map_err(NewRelicSendError::Request)?;
        Ok(response.status)
    }
}
