use std::collections::BTreeMap;
use std::future::Future;
use std::time::Duration;

use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use indexmap::IndexMap;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use reqwest::{Method, Url};
use secrecy::{ExposeSecret as _, SecretString};

use crate::straiker::delivery::settings::{ApiVersion, StraikerSettings};
use crate::straiker_webhook::failure::WebhookFailure;

const RESERVED_HEADERS: [&str; 3] = ["authorization", "content-type", "x-straiker-webhook-format"];

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

pub struct WebhookHeaders {
    pub authorization: SecretString,
    pub values: BTreeMap<String, String>,
}

pub trait WebhookPost: Sync {
    fn post(
        &self,
        request: WebhookRequest<'_>,
    ) -> impl Future<Output = Result<WebhookReply, WebhookFailure>> + Send;
}

#[must_use]
pub fn compose(
    settings: &StraikerSettings,
    extra: &IndexMap<&'static str, String>,
) -> WebhookHeaders {
    let mut values = BTreeMap::from([("Content-Type".to_owned(), "application/json".to_owned())]);
    if settings.api_version != ApiVersion::V3 {
        values.insert("X-Straiker-Webhook-Format".to_owned(), "litellm".to_owned());
    }
    for (name, value) in &settings.custom_headers {
        if !RESERVED_HEADERS.contains(&name.to_lowercase().as_str()) {
            values.insert(name.clone(), value.clone());
        }
    }
    for (name, value) in extra {
        values.insert((*name).to_owned(), value.clone());
    }
    let key = settings.api_key.expose_secret();
    WebhookHeaders {
        authorization: SecretString::from(format!("Bearer {key}")),
        values,
    }
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
        // httpx hands a 200 body to json.loads as bytes, which refuses bytes that are not UTF-8.
        let body = str::from_utf8(&response.body).map_or_else(|_| String::new(), String::from);
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

// httpx raises a RequestError for a header httpcore refuses, and the attempt loop retries that like a network error.
const fn unreachable(message: String) -> WebhookFailure {
    WebhookFailure {
        message,
        is_unreachable: true,
        retryable: true,
    }
}

// httpx raises UnicodeEncodeError, a ValueError, for a header it cannot encode as ASCII, which the attempt loop treats as permanent.
const fn permanent(message: String) -> WebhookFailure {
    WebhookFailure {
        message,
        is_unreachable: false,
        retryable: false,
    }
}

fn outbound_request(request: &WebhookRequest<'_>) -> Result<OutboundRequest, WebhookFailure> {
    let url = Url::parse(request.url)
        .map_err(|source| unreachable(format!("the webhook URL is not valid: {source}")))?;
    let mut headers = HeaderMap::new();
    for (name, value) in &request.headers.values {
        if !name.is_ascii() || !value.is_ascii() {
            return Err(permanent(format!("the {name} header is not ASCII")));
        }
        let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|source| {
            unreachable(format!("the {name} header name is not valid: {source}"))
        })?;
        let header_value = HeaderValue::from_bytes(value.as_bytes()).map_err(|source| {
            unreachable(format!("the {name} header value is not valid: {source}"))
        })?;
        headers.append(header_name, header_value);
    }
    let key = request.headers.authorization.expose_secret();
    if !key.is_ascii() {
        return Err(permanent(
            "the Authorization header is not ASCII".to_owned(),
        ));
    }
    let mut authorization = HeaderValue::from_bytes(key.as_bytes())
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
