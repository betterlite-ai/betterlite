use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use futures_util::StreamExt as _;
use futures_util::stream::BoxStream;
use http::Method;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use url::Url;

use crate::live::ProbeError;

pub type BodyStream = BoxStream<'static, Result<Vec<u8>, ProbeError>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HttpMethod {
    Post,
    Get,
}

pub struct OutboundHttp {
    pub method: HttpMethod,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub deadline: Duration,
}

pub struct HttpAnswer {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: BodyStream,
}

#[async_trait]
pub trait HttpTransport: Send + Sync {
    async fn send(&self, request: OutboundHttp) -> Result<HttpAnswer, ProbeError>;
}

#[derive(Clone)]
pub struct PooledHttp {
    pub client: ProxiedClient,
}

#[async_trait]
impl HttpTransport for PooledHttp {
    // the pooled client only reaches a live host, so no unit test can observe this method
    #[cfg_attr(test, mutants::skip)]
    async fn send(&self, request: OutboundHttp) -> Result<HttpAnswer, ProbeError> {
        let url = Url::parse(&request.url).map_err(ProbeError::Url)?;
        let method = match request.method {
            HttpMethod::Post => Method::POST,
            HttpMethod::Get => Method::GET,
        };
        let headers = signed_headers(request.headers)?;
        let body = if request.body.is_empty() {
            RequestBody::Empty
        } else {
            RequestBody::Json(request.body)
        };
        let answer = self
            .client
            .send(OutboundRequest {
                method,
                url,
                query: Vec::new(),
                headers,
                body,
                deadline: request.deadline,
            })
            .await
            .map_err(ProbeError::Transport)?;
        let status = answer.response.status().as_u16();
        let response_headers = header_pairs(answer.response.headers());
        let stream = answer
            .body_stream()
            .map(|result| result.map_err(ProbeError::Body))
            .boxed();
        Ok(HttpAnswer {
            status,
            headers: response_headers,
            body: stream,
        })
    }
}

pub(super) fn signed_headers(pairs: Vec<(String, String)>) -> Result<HeaderMap, ProbeError> {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        let header_name =
            HeaderName::from_bytes(name.as_bytes()).map_err(|source| ProbeError::HeaderName {
                name: Box::from(name.as_str()),
                source,
            })?;
        let header_value =
            HeaderValue::from_str(&value).map_err(|source| ProbeError::HeaderValue {
                name: Box::from(name.as_str()),
                source,
            })?;
        headers.insert(header_name, header_value);
    }
    Ok(headers)
}

pub(super) fn header_pairs(headers: &HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| {
            (
                name.to_string(),
                value.to_str().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}
