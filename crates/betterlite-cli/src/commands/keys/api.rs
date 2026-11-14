use http::Method;
use http::StatusCode;
use secrecy::SecretString;
use url::Url;

use betterlite_config::sections::settings::JsonValue;
use betterlite_redact::secret::Redactor;

use crate::commands::{Client, ClientError, Exchange, Payload, REQUEST_TIMEOUT, Request};

pub(super) struct Call<'request> {
    pub(super) method: Method,
    pub(super) base_url: &'request str,
    pub(super) path: &'request str,
    pub(super) query: &'request [(String, String)],
    pub(super) body: Option<Vec<u8>>,
    pub(super) api_key: Option<&'request SecretString>,
}

pub struct Reply {
    pub method: Method,
    pub url: String,
    pub status: u16,
    pub body: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error("HTTP {status} {reason} for {method} {url}")]
    Http {
        status: u16,
        reason: String,
        method: Method,
        url: String,
        body: Vec<u8>,
    },
    #[error("{text}")]
    Unauthorized { text: String },
    #[error("the response from {url} is not JSON: {source}")]
    Response {
        url: String,
        #[source]
        source: serde_json::Error,
    },
}

pub(super) fn call(transport: &Client, request: Call<'_>) -> Result<Reply, ClientError> {
    let Call {
        method,
        base_url,
        path,
        query,
        body,
        api_key,
    } = request;
    let url = build_url(base_url, path, query)?;
    let url_text = url.to_string();
    let headers = vec![(
        String::from("content-type"),
        String::from("application/json"),
    )];
    let payload = body.map_or(Payload::Empty, Payload::Raw);
    let exchange = Exchange {
        transport: &transport.http,
        request: Request {
            method: method.clone(),
            url,
            headers,
            payload,
        },
        timeout: REQUEST_TIMEOUT,
        authorization: api_key,
    };
    let response = transport.exchange(exchange)?;
    Ok(Reply {
        method,
        url: url_text,
        status: response.status,
        body: response.body,
    })
}

fn build_url(base_url: &str, path: &str, query: &[(String, String)]) -> Result<Url, ClientError> {
    let full = format!("{base_url}{path}");
    let mut url = Url::parse(&full).map_err(|source| ClientError::Url { url: full, source })?;
    if !query.is_empty() {
        url.query_pairs_mut().extend_pairs(
            query
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_str())),
        );
    }
    Ok(url)
}

pub fn interpret(reply: &Reply, redactor: Option<&Redactor>) -> Result<JsonValue, ApiError> {
    if (400..=599).contains(&reply.status) {
        let reason = StatusCode::from_u16(reply.status)
            .ok()
            .and_then(|status| status.canonical_reason())
            .unwrap_or_default();
        let text = format!(
            "HTTP {} {reason} for {} {}",
            reply.status, reply.method, reply.url
        );
        if reply.status == 401
            && let Some(masker) = redactor
        {
            return Err(ApiError::Unauthorized {
                text: masker.redact_string(&text),
            });
        }
        return Err(ApiError::Http {
            status: reply.status,
            reason: reason.to_owned(),
            method: reply.method.clone(),
            url: reply.url.clone(),
            body: reply.body.clone(),
        });
    }
    serde_json::from_slice(&reply.body).map_err(|source| ApiError::Response {
        url: reply.url.clone(),
        source,
    })
}
