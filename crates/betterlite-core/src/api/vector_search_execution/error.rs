use std::collections::BTreeMap;
use std::error::Error;
use std::str::from_utf8;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::Json;
use betterlite_openai::vector_stores::error::VectorStoreError;
use http::header::{InvalidHeaderName, InvalidHeaderValue};
use serde_json::Number;
use thiserror::Error;

use super::config::SearchConfig;
use crate::api::vector_store_http_write::error::{
    BaseLlmException, ClientError, FailureFacts, ResponseFacts, handle_error, header_text,
};

pub const TIMEOUT_MESSAGE: &str = "Vector store search exceeded the caller timeout.";

// The handler's own failures; a provider's exception keeps its status, message and headers
#[derive(Debug, Error)]
pub enum SearchError {
    #[error(transparent)]
    Provider(#[from] BaseLlmException),
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error(transparent)]
    Logging(#[from] PrePostError),
    #[error(transparent)]
    Request(#[from] VectorStoreError),
    #[error("the search request URL {url} is not a URL")]
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
    #[error("the search request body could not be encoded")]
    Encode(#[source] serde_json::Error),
    #[error("the {field} field must be {expected}")]
    Field {
        field: &'static str,
        expected: &'static str,
    },
    #[error(transparent)]
    Signing(#[from] SigningFailure),
    #[error(transparent)]
    Direct(#[from] DirectFailure),
}

// B595's off-loop signing failure, boxed so the core keeps no dependency on the signing crate
#[derive(Debug, Error)]
#[error("the search request could not be signed: {source}")]
pub struct SigningFailure {
    #[source]
    pub source: Box<dyn Error + Send + Sync>,
}

// A direct store's own exception, which the handler passes through without HTTP error conversion
#[derive(Debug, Error)]
#[error("{message}")]
pub struct DirectFailure {
    pub message: String,
    #[source]
    pub source: Box<dyn Error + Send + Sync>,
}

impl DirectFailure {
    #[must_use]
    pub fn new(source: Box<dyn Error + Send + Sync>) -> Self {
        let message = source.to_string();
        Self { message, source }
    }
}

// B2402's converter decides the status, text and headers; this config only picks the exception class they land in
#[must_use]
pub(super) fn convert_error(facts: &FailureFacts, config: &dyn SearchConfig) -> BaseLlmException {
    let converted = handle_error(facts, None);
    let mut error =
        config.error_class(&converted.message, converted.status_code, converted.headers);
    error.status_code_is_synthesized = converted.status_code_is_synthesized;
    error
}

// An httpx.HTTPStatusError carries the masked response: its text and its headers less content-encoding, with content-length recomputed
pub(super) fn reply_facts(reply: &UpstreamResponse) -> FailureFacts {
    let text = from_utf8(&reply.body).map_or_else(
        |_| format!("{} bytes that are not UTF-8", reply.body.len()),
        ToOwned::to_owned,
    );
    let mut headers = error_headers(&reply.headers);
    // httpx re-encodes the decoded body into the masked response, so a non-empty one carries a new content-length
    if !reply.body.is_empty() {
        let length = reply.body.len().to_string();
        headers.insert("content-length".to_owned(), Json::Text(length));
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

fn error_headers(headers: &http::HeaderMap) -> BTreeMap<String, Json> {
    headers
        .iter()
        .filter(|(name, _)| !matches!(name.as_str(), "content-encoding" | "content-length"))
        .map(|(name, value)| (name.as_str().to_owned(), Json::Text(header_text(value))))
        .collect()
}

pub(super) fn transport_facts(error: &TransportError) -> FailureFacts {
    FailureFacts {
        response_status: None,
        status: None,
        text: None,
        headers: None,
        response: None,
        display: error.to_string(),
    }
}
