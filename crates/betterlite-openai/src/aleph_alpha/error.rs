use betterlite_http::proxy::transport_error::TransportError;
use betterlite_tokenizer::engine::error::EngineError;
use http::header::InvalidHeaderValue;
use thiserror::Error;
use url::ParseError;

#[derive(Debug, Error)]
pub enum AlephAlphaError {
    // LiteLLM's AlephAlphaError carries the status the reply arrived with, whatever that status was
    #[error("{message}")]
    Provider { status_code: u16, message: String },
    #[error("message {index} content is not plain text")]
    Content { index: usize },
    #[error("the stream parameter is not a boolean")]
    StreamType,
    #[error("the api_base is not a URL")]
    Endpoint {
        #[source]
        source: ParseError,
    },
    #[error("the {name} header value cannot be carried in a request")]
    Header {
        name: &'static str,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the request body could not be written as JSON")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the reply carries no completions, so no first choice holds a usage count")]
    NoChoices,
    #[error("the token count does not fit the reply")]
    Usage,
    #[error("the tokenizer failed")]
    Tokenize {
        #[source]
        source: EngineError,
    },
    #[error("a streamed line is longer than {limit} bytes")]
    LineTooLong { limit: usize },
    #[error("the provider could not be reached")]
    Transport {
        #[source]
        source: TransportError,
    },
}

impl AlephAlphaError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Provider { status_code, .. } => Some(*status_code),
            Self::Content { .. }
            | Self::StreamType
            | Self::Endpoint { .. }
            | Self::Header { .. }
            | Self::Encode { .. }
            | Self::NoChoices
            | Self::Usage
            | Self::Tokenize { .. }
            | Self::LineTooLong { .. }
            | Self::Transport { .. } => None,
        }
    }
}
