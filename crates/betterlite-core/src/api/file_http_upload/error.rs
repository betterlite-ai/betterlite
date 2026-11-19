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

// A provider transform that fails before any request is made raises its own error, so the reason
// travels as a source, the way BaseVectorStoreConfig.get_complete_url's does
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

// _handle_error's conversion, over the facts B2402 reads off the caught exception
#[must_use]
pub fn converted(facts: &FailureFacts) -> UploadError {
    UploadError::Provider(handle_error(facts, None))
}

// Anything that is not an HTTPStatusError keeps only its display text and becomes a synthesized 500
#[must_use]
pub const fn display_failure(display: String) -> FailureFacts {
    FailureFacts {
        response_status: None,
        status: None,
        text: None,
        headers: None,
        response: None,
        display,
    }
}

// A ValueError the handler raises itself: its str() is the whole message, and the status is synthesized
#[must_use]
pub const fn value_failure(message: String) -> FailureFacts {
    FailureFacts {
        response_status: None,
        status: None,
        text: Some(message),
        headers: None,
        response: None,
        display: String::new(),
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
    // httpx re-encodes the decoded body into the masked response, so a non-empty one carries a new length
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

// A transport failure carries no status, so the converter synthesizes the 500 from its text
#[must_use]
pub fn transport_failure(error: &TransportError) -> FailureFacts {
    display_failure(error.to_string())
}
