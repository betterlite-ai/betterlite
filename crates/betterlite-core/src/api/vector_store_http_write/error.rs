use std::collections::BTreeMap;
use std::error::Error;

use betterlite_config::tls::TlsPolicyError;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::pool::PoolError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::Json;
use http::header::{InvalidHeaderName, InvalidHeaderValue};
use serde_json::Number;
use thiserror::Error;

use super::config::WriteConfig;

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

#[derive(Debug, Clone, PartialEq)]
pub struct ResponseFacts {
    pub status_code: i64,
    pub text: Option<String>,
    pub headers: Option<HeaderMap>,
}

// Everything llm_http_handler.py reads off the caught exception before it picks a status, a text and a header map
#[derive(Debug, Clone, PartialEq)]
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
        std::string::ToString::to_string,
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
        error_headers = response.headers.clone();
    }
    if let Some(response) = &facts.response
        && let Some(text) = &response.text
    {
        error_text = text.clone();
    }
    let error_headers = error_headers.unwrap_or_default();
    let mut error = match config {
        Some(config) => config.error_class(&error_text, code, error_headers),
        None => BaseLlmException {
            status_code: code,
            message: error_text,
            headers: error_headers,
            status_code_is_synthesized: false,
        },
    };
    if synthesized {
        error.status_code_is_synthesized = true;
    }
    error
}
