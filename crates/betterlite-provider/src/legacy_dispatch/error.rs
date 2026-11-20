use std::io::Error as IoError;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::aleph_alpha::error::AlephAlphaError;
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
    #[error("the request body could not be written as JSON")]
    Encode {
        #[source]
        source: JsonError,
    },
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

pub(super) fn operation(error: LegacyError) -> OperationError {
    match error {
        LegacyError::Encode { source } => OperationError::Encode { source },
        other @ (LegacyError::Content { .. }
        | LegacyError::CustomOutput
        | LegacyError::ExtraBodyNotObject
        | LegacyError::Endpoint { .. }
        | LegacyError::Reply { .. }
        | LegacyError::Settings { .. }) => undecodable(other.to_string()),
    }
}

// Aleph Alpha keeps the status its reply arrived with, and the transport errors keep their own class
pub(super) fn aleph(error: AlephAlphaError) -> OperationError {
    match error {
        AlephAlphaError::Provider {
            status_code,
            message,
        } => OperationError::Provider {
            response: Box::new(UpstreamResponse {
                status: status_code,
                headers: HeaderMap::new(),
                body: message.into_bytes(),
            }),
        },
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
