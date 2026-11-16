use std::collections::BTreeMap;

use betterlite_anthropic::anthropic_capability_catalog::thinking::ThinkingError;
use betterlite_anthropic::capabilities::reasoning::ReasoningEffortError;
use betterlite_anthropic::output_schema::SchemaError;
use betterlite_anthropic::tools::ToolsError;
use betterlite_openai::error::{OpenAiError, ProviderError, ProviderErrorParts};
use thiserror::Error;

use crate::databricks_auth::error::DatabricksError;

// LiteLLM's Databricks chat surface failures: a typed body error keeps the upstream status, a
// malformed reply or chunk is a 500 except for the KeyError path the chunk parser turns into a 400.
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
    #[error("{message}")]
    Response {
        status_code: u16,
        message: String,
        headers: BTreeMap<String, String>,
    },
    #[error("{message}")]
    BadRequest { message: String },
    #[error("{message}")]
    Shape { message: String },
    #[error("KeyError: '{key}', Got unexpected response from Databricks: {chunk}")]
    ChunkKey { key: &'static str, chunk: String },
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
    // The shape errors the Databricks transforms raise when a declared type does not hold
    pub(crate) fn shape(message: impl Into<String>) -> Self {
        Self::Shape {
            message: message.into(),
        }
    }

    pub(crate) fn missing(owner: &str, name: &str) -> Self {
        Self::shape(format!("the {name} of a {owner} is missing"))
    }

    pub(crate) fn wrong_type(owner: &str, expected: &str) -> Self {
        Self::shape(format!("the {owner} must be {expected}"))
    }

    // chunk_parser's KeyError path: the 400 that names the key and the whole chunk
    pub(crate) fn chunk_key(key: &'static str, rendered: &str) -> Self {
        Self::ChunkKey {
            key,
            chunk: rendered.to_owned(),
        }
    }

    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::Databricks(error) => error.status_code,
            Self::Tools(error) => match error.status_code() {
                Some(status) => status,
                None => 400,
            },
            Self::Reasoning(_)
            | Self::InvalidEffort { .. }
            | Self::ChunkKey { .. }
            | Self::BadRequest { .. } => 400,
            Self::Response { status_code, .. } => *status_code,
            Self::Schema(_)
            | Self::Thinking(_)
            | Self::Base(_)
            | Self::Shape { .. }
            | Self::Encode(_)
            | Self::Decode(_) => 500,
        }
    }
}

impl From<DatabricksChatError> for ProviderError {
    fn from(error: DatabricksChatError) -> Self {
        let headers: Vec<(String, String)> = match &error {
            Self::Databricks(inner) => return Self::from(inner.clone()),
            Self::Response { headers, .. } => headers
                .iter()
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect(),
            Self::Tools(_)
            | Self::Schema(_)
            | Self::Reasoning(_)
            | Self::Thinking(_)
            | Self::Base(_)
            | Self::BadRequest { .. }
            | Self::Shape { .. }
            | Self::ChunkKey { .. }
            | Self::InvalidEffort { .. }
            | Self::Encode(_)
            | Self::Decode(_) => Vec::new(),
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
