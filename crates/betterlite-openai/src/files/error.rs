use std::str;

use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use serde_json::Value;
use thiserror::Error;

// The SDK exception an upstream 4xx or 5xx becomes; the lifecycle wrappers pass it through untouched
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusError {
    pub status: u16,
    pub message: String,
    pub error_type: Option<String>,
    pub param: Option<String>,
    pub code: Option<String>,
}

impl StatusError {
    // the SDK's status-error mapping: a JSON body keeps its "error" object, any other body is the message
    #[must_use]
    pub fn from_response(status: u16, body: &[u8]) -> Self {
        let trimmed = str::from_utf8(body).unwrap_or_default().trim();
        let Ok(parsed) = serde_json::from_str::<Value>(trimmed) else {
            let message = if trimmed.is_empty() {
                format!("Error code: {status}")
            } else {
                trimmed.to_owned()
            };
            return Self {
                status,
                message,
                error_type: None,
                param: None,
                code: None,
            };
        };
        let data = parsed.get("error").unwrap_or(&parsed);
        let field = |name: &str| data.get(name).and_then(Value::as_str).map(str::to_owned);
        Self {
            status,
            message: format!("Error code: {status} - {parsed}"),
            error_type: field("type"),
            param: field("param"),
            code: field("code"),
        }
    }
}

#[derive(Debug, Error)]
pub enum FilesError {
    #[error(
        "OpenAI client is not initialized. Make sure api_key is passed or OPENAI_API_KEY is set in the environment."
    )]
    NotInitialized,
    #[error(
        "OpenAI client is not an instance of AsyncOpenAI. Make sure you passed an AsyncOpenAI client."
    )]
    NotAsync,
    #[error("Expected a non-empty value for `{field}` but received ''")]
    EmptyValue { field: &'static str },
    #[error("Constructed path '{path}' contains dot-segment '{segment}' which is not allowed")]
    DotSegment { path: String, segment: String },
    #[error("the api_base is not a URL")]
    BaseUrl {
        #[source]
        source: url::ParseError,
    },
    #[error("a request header name or value is not valid")]
    Header {
        #[source]
        source: http::Error,
    },
    #[error("{}", .0.message)]
    Status(Box<StatusError>),
    #[error("the file reply is not a file object")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the request could not be sent")]
    Transport {
        #[source]
        source: TransportError,
    },
    #[error("the file content stream broke")]
    Body {
        #[source]
        source: HttpError,
    },
}
