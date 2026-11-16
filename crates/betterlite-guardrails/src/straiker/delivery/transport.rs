use std::future::Future;
use std::time::Duration;

use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use reqwest::{Method, Url};
use secrecy::ExposeSecret as _;

use crate::straiker::delivery::headers::WebhookHeaders;
use crate::straiker_webhook::failure::WebhookFailure;

pub struct WebhookRequest<'request> {
    pub url: &'request str,
    pub body: &'request str,
    pub headers: &'request WebhookHeaders,
    pub timeout: Duration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WebhookReply {
    pub status: u16,
    pub body: String,
}

pub trait WebhookPost: Sync {
    fn post(
        &self,
        request: WebhookRequest<'_>,
    ) -> impl Future<Output = Result<WebhookReply, WebhookFailure>> + Send;
}

// LiteLLM's shared async httpx client: one pooled connection, no retries of its own, the caller's timeout.
pub struct PooledPost {
    pub client: ProxiedClient,
}

impl PooledPost {
    async fn exchange(&self, request: WebhookRequest<'_>) -> Result<WebhookReply, WebhookFailure> {
        let outbound = outbound_request(&request)?;
        let response = self
            .client
            .send_buffered(outbound)
            .await
            .map_err(|source| unreachable(source.to_string()))?;
        let body = String::from_utf8(response.body).map_err(|source| WebhookFailure {
            message: format!("the response body is not UTF-8: {source}"),
            is_unreachable: false,
            retryable: false,
        })?;
        Ok(WebhookReply {
            status: response.status,
            body,
        })
    }
}

impl WebhookPost for PooledPost {
    fn post(
        &self,
        request: WebhookRequest<'_>,
    ) -> impl Future<Output = Result<WebhookReply, WebhookFailure>> + Send {
        self.exchange(request)
    }
}

// httpx raises a RequestError for a URL or header it cannot send, and the attempt loop retries that like a network error.
fn unreachable(message: String) -> WebhookFailure {
    WebhookFailure {
        message,
        is_unreachable: true,
        retryable: true,
    }
}

fn outbound_request(request: &WebhookRequest<'_>) -> Result<OutboundRequest, WebhookFailure> {
    let url = Url::parse(request.url)
        .map_err(|source| unreachable(format!("the webhook URL is not valid: {source}")))?;
    let mut headers = HeaderMap::new();
    for (name, value) in &request.headers.values {
        let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|source| {
            unreachable(format!("the {name} header name is not valid: {source}"))
        })?;
        let header_value = HeaderValue::from_bytes(value.as_bytes()).map_err(|source| {
            unreachable(format!("the {name} header value is not valid: {source}"))
        })?;
        headers.insert(header_name, header_value);
    }
    let mut authorization = HeaderValue::from_bytes(
        request.headers.authorization.expose_secret().as_bytes(),
    )
    .map_err(|source| unreachable(format!("the Authorization value is not valid: {source}")))?;
    authorization.set_sensitive(true);
    headers.insert(AUTHORIZATION, authorization);
    Ok(OutboundRequest {
        method: Method::POST,
        url,
        query: Vec::new(),
        headers,
        body: RequestBody::Json(request.body.as_bytes().to_vec()),
        deadline: request.timeout,
    })
}
