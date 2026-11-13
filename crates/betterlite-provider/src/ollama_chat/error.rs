use std::collections::BTreeMap;

use betterlite_openai::error::{ProviderError, ProviderErrorParts};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum OllamaError {
    // the class get_error_class returns and the chunk parser raises: Ollama's status, message and headers
    #[error("{message}")]
    Api {
        status_code: u16,
        message: String,
        headers: BTreeMap<String, String>,
    },
    #[error("the Ollama JSON has the wrong shape: {source}")]
    Shape {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Ollama reply field {field} has the wrong type")]
    ReplyField { field: &'static str },
    #[error("the Ollama stream chunk is missing {field}")]
    ChunkMissing { field: &'static str },
    #[error("the Ollama stream chunk field {field} has the wrong type")]
    ChunkField { field: &'static str },
    #[error("the Ollama tool call {field} has the wrong shape")]
    Arguments { field: &'static str },
    #[error("the Ollama parameter {field} has the wrong type")]
    Param { field: &'static str },
    #[error("message {index} has a {field} that is not the declared type")]
    RequestField { index: usize, field: &'static str },
    #[error("message {index} has a {field} with the wrong shape: {source}")]
    RequestFieldShape {
        index: usize,
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("message {index} has tool call arguments that are not JSON: {source}")]
    RequestToolArguments {
        index: usize,
        #[source]
        source: serde_json::Error,
    },
}

impl OllamaError {
    // LiteLLM lets a malformed request escape as a 500; betterlite answers the client error with 400 instead
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::Api { status_code, .. } => Some(*status_code),
            Self::ChunkMissing { .. }
            | Self::Param { .. }
            | Self::RequestField { .. }
            | Self::RequestFieldShape { .. }
            | Self::RequestToolArguments { .. } => Some(400),
            Self::Shape { .. }
            | Self::ReplyField { .. }
            | Self::ChunkField { .. }
            | Self::Arguments { .. } => None,
        }
    }
}

impl From<OllamaError> for ProviderError {
    fn from(error: OllamaError) -> Self {
        let status_code = error.status().unwrap_or(500);
        let message = error.to_string();
        let headers = match &error {
            OllamaError::Api { headers, .. } => headers.clone(),
            OllamaError::Shape { .. }
            | OllamaError::ReplyField { .. }
            | OllamaError::ChunkMissing { .. }
            | OllamaError::ChunkField { .. }
            | OllamaError::Arguments { .. }
            | OllamaError::Param { .. }
            | OllamaError::RequestField { .. }
            | OllamaError::RequestFieldShape { .. }
            | OllamaError::RequestToolArguments { .. } => BTreeMap::new(),
        };
        let pairs: Vec<(String, String)> = headers.into_iter().collect();
        Self::from(ProviderErrorParts {
            status_code,
            message,
            headers: (!pairs.is_empty()).then_some(pairs),
            request: None,
            response: None,
            body: None,
            status_code_is_synthesized: false,
        })
    }
}
