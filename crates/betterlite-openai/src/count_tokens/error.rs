use betterlite_http::proxy::transport_error::TransportError;
use thiserror::Error;

use crate::error::{ProviderError, ProviderErrorParts};

#[derive(Debug, Error)]
pub enum CountTokensError {
    #[error("{}", .0.message)]
    Api(Box<ProviderError>),
    #[error("model parameter is required")]
    ModelRequired,
    #[error("input parameter is required")]
    InputRequired,
    #[error("the endpoint URL is not valid: {source}")]
    Endpoint {
        #[source]
        source: url::ParseError,
    },
    #[error("the Authorization header cannot carry this API key")]
    InvalidApiKey,
    #[error("the request body does not encode: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the request failed: {source}")]
    Transport {
        #[source]
        source: TransportError,
    },
    #[error("the upstream answered with status {status}")]
    Status { status: u16, text: String },
    #[error("the upstream answered with status {status} and a body that is not UTF-8 text")]
    BodyNotUtf8 { status: u16 },
    #[error("the answer is not JSON: {source}")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the answer is not a JSON object")]
    NotAnObject,
    #[error("input_tokens is not an integer")]
    InputTokensType,
    #[error("a function tool is not a valid chat function: {source}")]
    Tool {
        #[source]
        source: serde_json::Error,
    },
    #[error("a function tool's function is not an object")]
    ToolNotObject,
}

impl CountTokensError {
    #[must_use]
    pub fn openai_error(status_code: u16, message: String) -> Self {
        Self::Api(Box::new(ProviderError::from(ProviderErrorParts {
            status_code,
            message,
            headers: None,
            request: None,
            response: None,
            body: None,
            status_code_is_synthesized: false,
        })))
    }

    // Any error that is not an OpenAIError reaches the counter as a bare exception, which it reports as a 500
    #[must_use]
    pub fn status_and_message(&self) -> (u16, String) {
        if let Self::Api(api) = self {
            (api.status_code, api.message.clone())
        } else {
            (500, self.to_string())
        }
    }

    // LiteLLM's handler lets these two escape its except clauses, so the counter reports them as a bare exception
    #[must_use]
    pub fn into_openai_error(self) -> Self {
        match self {
            Self::Api(_) | Self::Tool { .. } | Self::ToolNotObject | Self::InputTokensType => self,
            Self::Status { status, text } => Self::openai_error(status, text),
            Self::BodyNotUtf8 { status } => Self::openai_error(status, self.to_string()),
            Self::ModelRequired
            | Self::InputRequired
            | Self::Endpoint { .. }
            | Self::InvalidApiKey
            | Self::Encode { .. }
            | Self::Transport { .. }
            | Self::Decode { .. }
            | Self::NotAnObject => {
                Self::openai_error(500, format!("CountTokens processing error: {self}"))
            }
        }
    }
}
