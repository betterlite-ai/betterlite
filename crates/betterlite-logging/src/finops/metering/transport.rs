use std::error::Error;
use std::future::Future;
use std::string::FromUtf8Error;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use http::HeaderMap;
use http::Method;
use http::header::{AUTHORIZATION, HeaderValue, InvalidHeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

pub const EVENT_PATH: &str = "/api/v1/events";
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
// LiteLLM falls back to COMPLETION_HTTP_FALLBACK_SECONDS when litellm_settings.request_timeout is unset
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(600);
const RESPONSE_BODY_BYTES: usize = 1024 * 1024;
const RESPONSE_HEADER_BYTES: usize = 0x0001_0000;

// LiteLLM appends api/v1/events straight after a trailing slash and otherwise adds the slash
#[must_use]
pub fn events_url(base: &str) -> String {
    base.strip_suffix('/').map_or_else(
        || format!("{base}{EVENT_PATH}"),
        |trimmed| format!("{trimmed}{EVENT_PATH}"),
    )
}

#[derive(Debug)]
pub struct MeteringPost<'request> {
    pub url: Url,
    pub bearer: &'request SecretString,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

#[derive(Debug)]
pub struct MeteringResponse {
    pub status: u16,
    pub body: String,
}

pub trait MeteringSender: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn post(
        &self,
        post: MeteringPost<'_>,
    ) -> impl Future<Output = Result<MeteringResponse, Self::Error>> + Send;
}

#[derive(Debug)]
pub struct HttpMeteringSender {
    pub client: ProxiedClient,
    pub deadline: Duration,
}

#[derive(Debug, thiserror::Error)]
pub enum HttpSenderError {
    #[error("the metering request has a header value HTTP cannot carry")]
    Header(#[source] InvalidHeaderValue),
    #[error("the metering request failed")]
    Transport(#[source] TransportError),
    #[error("the metering response body is not UTF-8")]
    Body(#[source] FromUtf8Error),
}

impl HttpMeteringSender {
    pub fn new(policy: &Arc<ProxyPolicy>, timeout: Duration) -> Result<Self, HttpError> {
        let limits = TransportLimits {
            connect_timeout: CONNECT_TIMEOUT,
            read_timeout: timeout,
            body_bytes: RESPONSE_BODY_BYTES,
            header_bytes: RESPONSE_HEADER_BYTES,
        };
        Ok(Self {
            client: ProxiedClient::build(base_builder(), policy, limits)?,
            deadline: timeout,
        })
    }
}

impl MeteringSender for HttpMeteringSender {
    type Error = HttpSenderError;

    async fn post(&self, post: MeteringPost<'_>) -> Result<MeteringResponse, HttpSenderError> {
        let authorization =
            HeaderValue::try_from(format!("Bearer {}", post.bearer.expose_secret()))
                .map_err(HttpSenderError::Header)?;
        let content_type =
            HeaderValue::try_from(post.content_type).map_err(HttpSenderError::Header)?;
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, authorization);
        let request = OutboundRequest {
            method: Method::POST,
            url: post.url,
            query: Vec::new(),
            headers,
            body: RequestBody::Bytes {
                content_type,
                data: post.body,
            },
            deadline: self.deadline,
        };
        let response = self
            .client
            .send_buffered(request)
            .await
            .map_err(HttpSenderError::Transport)?;
        let body = String::from_utf8(response.body).map_err(HttpSenderError::Body)?;
        Ok(MeteringResponse {
            status: response.status,
            body,
        })
    }
}
