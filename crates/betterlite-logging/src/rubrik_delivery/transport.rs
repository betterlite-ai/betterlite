use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use http::Method;
use http::header::HeaderMap;
use thiserror::Error;
use url::{ParseError, Url};

// httpx.Timeout(5.0, connect=2.0) on the moderation client
pub const MODERATION_TIMEOUT: Timeout = Timeout {
    total: Duration::from_secs(5),
    connect: Duration::from_secs(2),
};
// LiteLLM's default cached logging client: COMPLETION_HTTP_FALLBACK_SECONDS and a 5-second connect
pub const LOGGING_TIMEOUT: Timeout = Timeout {
    total: Duration::from_secs(600),
    connect: Duration::from_secs(5),
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timeout {
    pub total: Duration,
    pub connect: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientTier {
    Moderation,
    Logging,
}

#[derive(Clone, Debug)]
pub struct JsonPost {
    pub tier: ClientTier,
    pub url: String,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
    pub timeout: Timeout,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransportResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

#[derive(Debug, Error)]
pub enum TransportFailure {
    #[error("the request URL is not valid")]
    Url {
        #[source]
        source: ParseError,
    },
    #[error("the request could not be completed")]
    Request {
        #[source]
        source: TransportError,
    },
}

#[async_trait]
pub trait Transport: Send + Sync {
    async fn post_json(&self, request: JsonPost) -> Result<TransportResponse, TransportFailure>;
}

#[derive(Debug, Error)]
pub enum ClientBuildError {
    #[error("the Rubrik HTTP client could not be built")]
    Client {
        #[source]
        source: HttpError,
    },
}

#[derive(Debug, Clone)]
pub struct HttpTransport {
    moderation: ProxiedClient,
    logging: ProxiedClient,
}

impl HttpTransport {
    pub fn new(
        policy: &Arc<ProxyPolicy>,
        moderation: Timeout,
        logging: Timeout,
    ) -> Result<Self, ClientBuildError> {
        Ok(Self {
            moderation: client(policy, moderation)?,
            logging: client(policy, logging)?,
        })
    }
}

fn client(policy: &Arc<ProxyPolicy>, timeout: Timeout) -> Result<ProxiedClient, ClientBuildError> {
    let defaults = TransportLimits::DEFAULT;
    let limits = TransportLimits {
        connect_timeout: timeout.connect,
        read_timeout: timeout.total,
        body_bytes: defaults.body_bytes,
        header_bytes: defaults.header_bytes,
    };
    ProxiedClient::build(base_builder(), policy, limits)
        .map_err(|source| ClientBuildError::Client { source })
}

#[async_trait]
impl Transport for HttpTransport {
    async fn post_json(&self, request: JsonPost) -> Result<TransportResponse, TransportFailure> {
        let url = Url::parse(&request.url).map_err(|source| TransportFailure::Url { source })?;
        let outbound = OutboundRequest {
            method: Method::POST,
            url,
            query: Vec::new(),
            headers: request.headers,
            body: RequestBody::Json(request.body),
            deadline: request.timeout.total,
        };
        let client = match request.tier {
            ClientTier::Moderation => &self.moderation,
            ClientTier::Logging => &self.logging,
        };
        let response = client
            .send_buffered(outbound)
            .await
            .map_err(|source| TransportFailure::Request { source })?;
        Ok(TransportResponse {
            status: response.status,
            body: response.body,
        })
    }
}
