use betterlite_http::proxy::transport_error::TransportError;
use thiserror::Error;

use crate::endpoint_target::error::EndpointTargetError;
use crate::token_cache::error::CredentialCacheError;

#[derive(Debug, Error)]
pub enum GoogleCountError {
    #[error("the Google Gen AI Studio API answered HTTP {status}: {body}")]
    Status { status: u16, body: String },
    #[error("the request to Google Gen AI Studio failed: {source}")]
    Connection {
        #[source]
        source: TransportError,
    },
    #[error("the Google count reply is larger than {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("the Google count JSON at {path} is not valid: {source}")]
    Json {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("the countTokens URL cannot be parsed: {source}")]
    Url {
        #[source]
        source: url::ParseError,
    },
    #[error("a count request header is not valid")]
    Header {
        #[source]
        source: http::Error,
    },
    #[error("the count contents {problem}")]
    Contents { problem: &'static str },
    #[error("the functionResponse record at {path} is not the one Google declares: {source}")]
    FunctionResponse {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("the vertex count credential could not be resolved: {source}")]
    Credentials {
        #[source]
        source: CredentialCacheError,
    },
    #[error("the vertex count URL could not be built: {source}")]
    Target {
        #[source]
        source: EndpointTargetError,
    },
}

impl GoogleCountError {
    // LiteLLM answers the upstream status through APIError and every other count failure as a 500
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::Status { status, .. } => *status,
            Self::Connection { .. }
            | Self::ResponseTooLarge { .. }
            | Self::Json { .. }
            | Self::Url { .. }
            | Self::Header { .. }
            | Self::Contents { .. }
            | Self::FunctionResponse { .. }
            | Self::Credentials { .. }
            | Self::Target { .. } => 500,
        }
    }
}
