use std::collections::BTreeMap;
use std::error::Error;
use std::str::{Utf8Error, from_utf8};

use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::Json;
use http::header::{InvalidHeaderName, InvalidHeaderValue};
use serde_json::Number;
use thiserror::Error;

use crate::api::api_boundaries::ApiLimitError;
use crate::api::vector_store_http_write::error::{
    BaseLlmException, ClientError, FailureFacts, ResponseFacts, handle_error, header_text,
};

// A provider transform that fails before any request raises its own error, so it travels as a source
#[derive(Debug, Error)]
#[error(transparent)]
pub struct UrlFailure {
    pub source: Box<dyn Error + Send + Sync>,
}

#[derive(Debug, Error)]
#[error(transparent)]
pub struct TransformFailure {
    pub source: Box<dyn Error + Send + Sync>,
}

#[derive(Debug, Error)]
pub enum UploadError {
    #[error("api_base is required for create_file")]
    ApiBaseRequired,
    #[error(transparent)]
    Provider(#[from] BaseLlmException),
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error(transparent)]
    Logging(#[from] PrePostError),
    #[error(transparent)]
    Transport(#[from] TransportError),
    #[error(transparent)]
    Url(#[from] UrlFailure),
    #[error(transparent)]
    Transform(#[from] TransformFailure),
    #[error(transparent)]
    Limit(#[from] ApiLimitError),
    #[error("Unsupported transformed_request type: <class 'dict'>")]
    UnsupportedPlan,
    #[error("the upload method {method} is not an HTTP method")]
    Method { method: String },
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
    #[error("the upload body is not UTF-8")]
    Decode(#[source] Utf8Error),
    #[error("the request body could not be encoded")]
    Encode(#[source] serde_json::Error),
}

impl UploadError {
    // _handle_error's conversion, over the facts B2402 reads off the caught exception
    #[must_use]
    pub fn provider(facts: &FailureFacts) -> Self {
        Self::Provider(handle_error(facts, None))
    }
}

// The exception facts when no response exists: text is a ValueError's message, display a failure's text
#[must_use]
pub const fn failure_facts(text: Option<String>, display: String) -> FailureFacts {
    FailureFacts {
        response_status: None,
        status: None,
        text,
        headers: None,
        response: None,
        display,
    }
}

// An httpx.HTTPStatusError: the status and the decoded body survive, with content-length recomputed
#[must_use]
pub fn status_failure(status: u16, body: &[u8], headers: &http::HeaderMap) -> FailureFacts {
    let number = i64::from(status);
    let text = from_utf8(body).map_or_else(
        |_| format!("{} bytes that are not UTF-8", body.len()),
        ToOwned::to_owned,
    );
    let mut error_headers: BTreeMap<String, Json> = headers
        .iter()
        .filter(|(name, _value)| !matches!(name.as_str(), "content-encoding" | "content-length"))
        .map(|(name, value)| (name.as_str().to_owned(), Json::Text(header_text(value))))
        .collect();
    // httpx re-encodes the decoded body, so a non-empty one carries a new content-length
    if !body.is_empty() {
        error_headers.insert(
            "content-length".to_owned(),
            Json::Text(body.len().to_string()),
        );
    }
    FailureFacts {
        response_status: Some(number),
        status: Some(Json::Number(Number::from(status))),
        text: Some(text.clone()),
        headers: None,
        response: Some(ResponseFacts {
            status_code: number,
            text: Some(text),
            headers: Some(error_headers),
        }),
        display: String::new(),
    }
}
