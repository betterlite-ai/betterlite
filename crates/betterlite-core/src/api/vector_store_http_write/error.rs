use std::collections::BTreeMap;
use std::error::Error;
use std::str::from_utf8;
use std::time::Duration;

use betterlite_config::tls::TlsPolicyError;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::pool::PoolError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::Json;
use http::header::{InvalidHeaderName, InvalidHeaderValue};
use serde_json::Number;
use thiserror::Error;

use super::config::WriteConfig;
use super::service::ClientKind;

pub type HeaderMap = BTreeMap<String, Json>;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct BaseLlmException {
    pub status_code: u16,
    pub message: String,
    pub headers: HeaderMap,
    pub status_code_is_synthesized: bool,
}

// BaseVectorStoreConfig.get_complete_url raises a provider error of its own, so the reason travels as a source
#[derive(Debug, Error)]
#[error(transparent)]
pub struct UrlFailure {
    pub source: Box<dyn Error + Send + Sync>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseFacts {
    pub status_code: i64,
    pub text: Option<String>,
    pub headers: Option<HeaderMap>,
}

// Everything llm_http_handler.py reads off the caught exception before it picks a status, a text and a header map
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureFacts {
    pub response_status: Option<i64>,
    pub status: Option<Json>,
    pub text: Option<String>,
    pub headers: Option<HeaderMap>,
    pub response: Option<ResponseFacts>,
    pub display: String,
}

#[derive(Debug, Error)]
pub enum ClientError {
    #[error(transparent)]
    Tls(#[from] TlsPolicyError),
    #[error(transparent)]
    Pool(#[from] PoolError),
}

#[derive(Debug, Error)]
pub enum WriteError {
    #[error(transparent)]
    Provider(#[from] BaseLlmException),
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error(transparent)]
    Logging(#[from] PrePostError),
    #[error(transparent)]
    Segment(#[from] HttpError),
    #[error(transparent)]
    Url(#[from] UrlFailure),
    #[error("the request URL {url} is not a URL")]
    RequestUrl {
        url: String,
        #[source]
        source: url::ParseError,
    },
    #[error("the {name} header name is not a header name")]
    HeaderName {
        name: String,
        #[source]
        source: InvalidHeaderName,
    },
    #[error("the {name} header value is not a header value")]
    HeaderValue {
        name: String,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the request body could not be encoded")]
    Encode(#[source] serde_json::Error),
    #[error("the reply body is not valid JSON")]
    Reply(#[source] serde_json::Error),
}

#[must_use]
pub fn header_text(value: &http::HeaderValue) -> String {
    value.to_str().map_or_else(
        |_| format!("{} bytes that are not UTF-8", value.as_bytes().len()),
        ToOwned::to_owned,
    )
}

#[must_use]
pub fn header_map(headers: &http::HeaderMap) -> HeaderMap {
    headers
        .iter()
        .map(|(name, value)| (name.as_str().to_owned(), Json::Text(header_text(value))))
        .collect()
}

// The shared error converter: an HTTP status error keeps its response status and text, another exception its integer status, and anything else becomes a synthesized 500
#[must_use]
pub fn handle_error(facts: &FailureFacts, config: Option<&dyn WriteConfig>) -> BaseLlmException {
    let received = facts
        .response_status
        .map(|status| Json::Number(Number::from(status)))
        .or_else(|| facts.status.clone());
    let status_code = match &received {
        Some(Json::Number(number)) => number.as_u64().and_then(|value| u16::try_from(value).ok()),
        _ => None,
    };
    let synthesized = status_code.is_none();
    let code = status_code.unwrap_or(500);
    let mut error_text = match (&facts.response_status, &facts.response) {
        (Some(_), Some(response)) => response.text.clone().unwrap_or_default(),
        _ => facts.text.clone().unwrap_or_else(|| facts.display.clone()),
    };
    let mut error_headers = facts.headers.clone();
    if error_headers.is_none()
        && let Some(response) = &facts.response
    {
        error_headers.clone_from(&response.headers);
    }
    if let Some(response) = &facts.response
        && let Some(text) = &response.text
    {
        error_text.clone_from(text);
    }
    let headers = error_headers.unwrap_or_default();
    let mut error = match config {
        Some(policy) => policy.error_class(&error_text, code, headers),
        None => BaseLlmException {
            status_code: code,
            message: error_text,
            headers,
            status_code_is_synthesized: false,
        },
    };
    if synthesized {
        error.status_code_is_synthesized = true;
    }
    error
}

// An httpx.HTTPStatusError carries the masked response: its text and its headers less content-encoding, with content-length recomputed
#[must_use]
pub(crate) fn status_facts(reply: &UpstreamResponse) -> FailureFacts {
    let text = body_text(&reply.body);
    let mut headers = header_map(&reply.headers);
    headers.remove("content-encoding");
    headers.remove("content-length");
    // httpx re-encodes the decoded body into the masked response, so a non-empty one carries a new content-length
    if !reply.body.is_empty() {
        let length = reply.body.len().to_string();
        headers.insert(String::from("content-length"), Json::Text(length));
    }
    FailureFacts {
        response_status: Some(i64::from(reply.status)),
        status: Some(Json::Number(Number::from(reply.status))),
        text: Some(text.clone()),
        headers: None,
        response: Some(ResponseFacts {
            status_code: i64::from(reply.status),
            text: Some(text),
            headers: Some(headers),
        }),
        display: String::new(),
    }
}

// A caught failure with no response behind it: the text is the failure's own, and a missing status becomes a synthesized 500
#[must_use]
pub(crate) const fn display_facts(display: String) -> FailureFacts {
    FailureFacts {
        response_status: None,
        status: None,
        text: None,
        headers: None,
        response: None,
        display,
    }
}

// A caught httpx.TimeoutException becomes litellm.Timeout: status 408 and the HTTP client's own message
#[must_use]
pub(crate) fn timeout_facts(
    mode: ClientKind,
    timeout: Option<Duration>,
    elapsed: Duration,
) -> FailureFacts {
    let message = match mode {
        ClientKind::Sync => format!(
            "litellm.Timeout: Connection timed out after {} seconds.",
            timeout_text(timeout)
        ),
        ClientKind::Asynchronous => format!(
            "litellm.Timeout: Connection timed out. Timeout passed={}, time taken={}.{:03} seconds",
            timeout_text(timeout),
            elapsed.as_secs(),
            elapsed.subsec_millis(),
        ),
    };
    FailureFacts {
        response_status: None,
        status: Some(Json::Number(Number::from(408_u16))),
        text: Some(message),
        headers: Some(HeaderMap::new()),
        response: None,
        display: String::new(),
    }
}

// A binary body is named by its byte count, as httpx's text decoding fails on it
fn body_text(body: &[u8]) -> String {
    from_utf8(body).map_or_else(
        |_| format!("{} bytes that are not UTF-8", body.len()),
        ToOwned::to_owned,
    )
}

// The sync and async HTTP clients word the same timeout differently, and the sync one shows the argument as it was given
fn timeout_text(timeout: Option<Duration>) -> String {
    let Some(given) = timeout else {
        return String::from("None");
    };
    if given.subsec_nanos() == 0 {
        return given.as_secs().to_string();
    }
    given.as_secs_f64().to_string()
}
