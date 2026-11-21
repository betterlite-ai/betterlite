use std::io::Error as IoError;
use std::str;

use betterlite_error::native_errors_b::aleph_alpha::{
    AlephAlphaExceptionInput, map_aleph_alpha_exception,
};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::aleph_alpha::error::AlephAlphaError;
use betterlite_openai::aleph_alpha::reply::decode_aleph_reply;
use http::HeaderMap;
use serde_json::Error as JsonError;
use thiserror::Error;

use crate::operation::OperationError;

#[derive(Debug, Error)]
pub enum LegacyError {
    #[error("message {index} content is not plain text")]
    Content { index: usize },
    #[error("the custom endpoint answer is not a data array holding a first text output")]
    CustomOutput,
    #[error("extra_body must be an object so it can expand into the request body")]
    ExtraBodyNotObject,
    #[error("the api_base is not a URL")]
    Endpoint {
        #[source]
        source: url::ParseError,
    },
    #[error("the custom endpoint answer could not be read as JSON")]
    Reply {
        #[source]
        source: JsonError,
    },
    #[error("the settings carry a value main.py cannot apply: {message}")]
    Settings { message: String },
}

impl From<LegacyError> for OperationError {
    fn from(error: LegacyError) -> Self {
        undecodable(error.to_string())
    }
}

// A failure betterlite models nowhere else reaches the caller as an unreadable provider reply
pub(super) fn undecodable(message: String) -> OperationError {
    OperationError::Decode {
        operation: "chat",
        source: JsonError::io(IoError::other(message)),
    }
}

pub(super) fn encode(message: String) -> OperationError {
    OperationError::Encode {
        source: JsonError::io(IoError::other(message)),
    }
}

// LiteLLM maps every aleph_alpha failure through _map_aleph_alpha_exception before the proxy renders it
fn mapped_provider(status_code: u16, message: String, headers: HeaderMap) -> OperationError {
    let error_str = message.as_str();
    let mapped = map_aleph_alpha_exception(&AlephAlphaExceptionInput {
        model: String::new(),
        error_str: error_str.to_owned(),
        status_code: Some(status_code),
        message: None,
    });
    OperationError::Provider {
        response: Box::new(UpstreamResponse {
            status: mapped.map_or(status_code, |class| class.metadata().status_code),
            headers,
            body: message.into_bytes(),
        }),
    }
}

// the reply-level failures take the same mapping, and the transport errors keep their own class
pub(super) fn aleph(error: AlephAlphaError) -> OperationError {
    match error {
        AlephAlphaError::Provider {
            status_code,
            message,
        } => mapped_provider(status_code, message, HeaderMap::new()),
        AlephAlphaError::Header { name, source } => OperationError::InvalidHeader {
            name: name.to_owned(),
            source,
        },
        AlephAlphaError::Encode { source } => OperationError::Encode { source },
        AlephAlphaError::Transport { source } => OperationError::Http { source },
        other @ (AlephAlphaError::Content { .. }
        | AlephAlphaError::StreamType
        | AlephAlphaError::Endpoint { .. }
        | AlephAlphaError::NoChoices
        | AlephAlphaError::Usage
        | AlephAlphaError::Tokenize { .. }
        | AlephAlphaError::LineTooLong { .. }) => undecodable(other.to_string()),
    }
}

// a non-2xx reply becomes the AlephAlphaError LiteLLM raises from that reply, then takes the same mapping
pub(super) fn aleph_failure(error: OperationError) -> OperationError {
    let OperationError::Provider { response } = error else {
        return error;
    };
    let UpstreamResponse {
        status,
        headers,
        body,
    } = *response;
    let message = match decode_aleph_reply(status, &body) {
        Err(AlephAlphaError::Provider { message, .. }) => message,
        Err(other) => other.to_string(),
        // a completion body that arrived under a failure status, kept as the reply text
        Ok(_) => match str::from_utf8(&body) {
            Ok(text) => text.to_owned(),
            Err(_source) => String::from("the reply body is not valid UTF-8"),
        },
    };
    mapped_provider(status, message, headers)
}
