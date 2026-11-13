use http::Method;
use http::StatusCode;
use http::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

use betterlite_config::sections::settings::JsonValue;
use betterlite_redact::secret::Redactor;

use crate::commands::{Client, ClientError, PreparedRequest, REQUEST_TIMEOUT};

pub(in crate::commands) struct Call<'request> {
    pub(in crate::commands) method: Method,
    pub(in crate::commands) base_url: &'request str,
    pub(in crate::commands) path: &'request str,
    pub(in crate::commands) query: &'request [(String, String)],
    pub(in crate::commands) headers: HeaderMap,
    pub(in crate::commands) body: Option<Vec<u8>>,
    pub(in crate::commands) api_key: Option<&'request SecretString>,
}

pub(in crate::commands) struct Reply {
    pub(in crate::commands) method: Method,
    pub(in crate::commands) url: String,
    pub(in crate::commands) status: u16,
    pub(in crate::commands) body: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub(in crate::commands) enum ApiError {
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

pub(in crate::commands) fn call(
    transport: &Client,
    request: Call<'_>,
) -> Result<Reply, ClientError> {
    let Call {
        method,
        base_url,
        path,
        query,
        headers,
        body,
        api_key,
    } = request;
    let url = build_url(base_url, path, query)?;
    let header_map = authorize(headers, api_key)?;
    let url_text = url.to_string();
    let prepared = PreparedRequest {
        method: method.clone(),
        url,
        headers: header_map,
        body: body.unwrap_or_default(),
        timeout: REQUEST_TIMEOUT,
    };
    let response = transport
        .send_request(&transport.http, prepared)
        .map_err(ClientError::Transport)?;
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

fn authorize(
    mut headers: HeaderMap,
    api_key: Option<&SecretString>,
) -> Result<HeaderMap, ClientError> {
    if let Some(key) = api_key {
        let bearer = format!("Bearer {}", key.expose_secret());
        let mut value = HeaderValue::from_bytes(bearer.as_bytes()).map_err(|source| {
            ClientError::HeaderValue {
                name: String::from("authorization"),
                source,
            }
        })?;
        value.set_sensitive(true);
        let _previous = headers.insert(AUTHORIZATION, value);
    }
    Ok(headers)
}

pub(in crate::commands) fn interpret(
    reply: &Reply,
    redactor: Option<&Redactor>,
) -> Result<JsonValue, ApiError> {
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
            && let Some(redactor) = redactor
        {
            return Err(ApiError::Unauthorized {
                text: redactor.redact_string(&text),
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
