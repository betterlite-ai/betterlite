use betterlite_http::proxy::transport_error::TransportError;
use thiserror::Error;

// LiteLLM's AnthropicError carries the upstream status and the raw response text
#[derive(Debug, Error)]
pub enum ApiError {
    #[error("{text}")]
    Upstream { status_code: u16, text: String },
    #[error("CountTokens processing error: {source}")]
    Processing {
        #[from]
        source: CountTokensError,
    },
}

impl ApiError {
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::Upstream { status_code, .. } => *status_code,
            Self::Processing { .. } => 500,
        }
    }
}

#[derive(Debug, Error)]
pub enum CountTokensError {
    #[error("model parameter is required")]
    MissingModel,
    #[error("messages parameter is required")]
    MissingMessages,
    #[error("messages must be a list")]
    MessagesNotList,
    #[error("Message {index} must be a dictionary")]
    MessageNotDictionary { index: usize },
    #[error("Message {index} must have a 'role' field")]
    MessageMissingRole { index: usize },
    #[error("Message {index} must have a 'content' field")]
    MessageMissingContent { index: usize },
    #[error("the CountTokens response is not a JSON object")]
    NotAnObject,
    #[error("input_tokens is not an integer")]
    InputTokens,
    #[error("the CountTokens request body could not be encoded: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the CountTokens response could not be decoded: {source}")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the CountTokens endpoint is not a URL: {source}")]
    Endpoint {
        #[source]
        source: url::ParseError,
    },
    #[error("the {name} header name is not valid")]
    InvalidHeaderName { name: String },
    #[error("the {name} header value is not valid")]
    InvalidHeaderValue { name: String },
    #[error("the CountTokens request could not be sent: {source}")]
    Transport {
        #[source]
        source: TransportError,
    },
}
