use std::collections::BTreeMap;

use betterlite_anthropic::anthropic_capability_catalog::thinking::ThinkingError;
use betterlite_anthropic::capabilities::reasoning::ReasoningEffortError;
use betterlite_anthropic::output_schema::SchemaError;
use betterlite_anthropic::tools::ToolsError;
use betterlite_openai::error::{OpenAiError, ProviderError, ProviderErrorParts};
use betterlite_prompts::media_fetch::error::ImageFetchError;
use thiserror::Error;

use crate::databricks_auth::error::DatabricksError;

#[derive(Debug, Error)]
pub enum DatabricksChatError {
    #[error(transparent)]
    Databricks(#[from] DatabricksError),
    #[error(transparent)]
    Tools(#[from] ToolsError),
    #[error(transparent)]
    Schema(#[from] SchemaError),
    #[error(transparent)]
    Reasoning(#[from] ReasoningEffortError),
    #[error(transparent)]
    Thinking(#[from] ThinkingError),
    #[error(transparent)]
    Base(#[from] OpenAiError),
    #[error(transparent)]
    Fetch(#[from] ImageFetchError),
    #[error("{message}")]
    Response {
        status_code: u16,
        message: String,
        headers: BTreeMap<String, String>,
    },
    #[error("content block has type='file' but is missing the required 'file' field")]
    MissingFile,
    #[error("{message}")]
    Shape { message: String },
    #[error("the Databricks response is missing its {key}")]
    MissingKey { key: &'static str },
    #[error("KeyError: '{key}', Got unexpected response from Databricks: {chunk}")]
    ChunkKey { key: &'static str, chunk: String },
    #[error("the created of a Databricks stream chunk is null and no clock is available")]
    NullCreated,
    #[error(
        "Invalid reasoning_effort: {value}. Must be one of: 'minimal', 'low', 'medium', 'high', 'xhigh', 'max', 'none'"
    )]
    InvalidEffort { value: String },
    #[error("a Databricks value cannot be encoded as JSON")]
    Encode(#[source] serde_json::Error),
    #[error("a Databricks value cannot be read as JSON")]
    Decode(#[source] serde_json::Error),
}

impl DatabricksChatError {
    #[must_use]
    pub fn status_code(&self) -> u16 {
        match self {
            Self::Databricks(error) => error.status_code,
            Self::Tools(error) => error.status_code().unwrap_or(400),
            Self::Response { status_code, .. } => *status_code,
            Self::Reasoning(_)
            | Self::Fetch(_)
            | Self::InvalidEffort { .. }
            | Self::ChunkKey { .. }
            | Self::MissingFile => 400,
            Self::Schema(_)
            | Self::Thinking(_)
            | Self::Base(_)
            | Self::Shape { .. }
            | Self::MissingKey { .. }
            | Self::NullCreated
            | Self::Encode(_)
            | Self::Decode(_) => 500,
        }
    }
}

impl From<DatabricksChatError> for ProviderError {
    fn from(error: DatabricksChatError) -> Self {
        let headers: Vec<(String, String)> = match &error {
            DatabricksChatError::Databricks(inner) => return Self::from(inner.clone()),
            DatabricksChatError::Response { headers, .. } => headers
                .iter()
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect(),
            DatabricksChatError::Tools(_)
            | DatabricksChatError::Schema(_)
            | DatabricksChatError::Reasoning(_)
            | DatabricksChatError::Thinking(_)
            | DatabricksChatError::Base(_)
            | DatabricksChatError::Fetch(_)
            | DatabricksChatError::MissingFile
            | DatabricksChatError::Shape { .. }
            | DatabricksChatError::MissingKey { .. }
            | DatabricksChatError::ChunkKey { .. }
            | DatabricksChatError::NullCreated
            | DatabricksChatError::InvalidEffort { .. }
            | DatabricksChatError::Encode(_)
            | DatabricksChatError::Decode(_) => Vec::new(),
        };
        Self::from(ProviderErrorParts {
            status_code: error.status_code(),
            message: error.to_string(),
            headers: Some(headers).filter(|pairs| !pairs.is_empty()),
            request: None,
            response: None,
            body: None,
            status_code_is_synthesized: false,
        })
    }
}
