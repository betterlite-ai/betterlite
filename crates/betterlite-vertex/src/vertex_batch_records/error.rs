use std::str::Utf8Error;

use thiserror::Error;

use crate::gemini_usage::error::UsageError;
use crate::provider_metadata::timestamp::VertexTimestampError;

#[derive(Debug, Error)]
pub enum BatchRecordsError {
    #[error("input_file_id is required, but not provided")]
    InputFileIdRequired,
    #[error("the Vertex batch input file id is not valid UTF-8")]
    IdentifierUtf8(#[source] Utf8Error),
    #[error(
        "the Vertex batch input file id '{identifier}' contains no 'publishers/<publisher>/models/<model>' or 'endpoints/<numeric endpoint id>' path segment"
    )]
    ModelMissing { identifier: String },
    #[error(
        "Vertex AI batch jobs against a fine-tuned endpoint ('{model}') require vertex_project to build the endpoint resource name"
    )]
    ProjectRequired { model: String },
    #[error("the Vertex batch job state is null")]
    NullJobState,
    #[error("the Vertex batch job createTime is invalid")]
    CreateTime(#[source] VertexTimestampError),
    #[error("the Vertex batch job response is not valid JSON")]
    Decode(#[source] serde_json::Error),
    #[error("the Vertex batch row usage is invalid")]
    Usage(#[from] UsageError),
}

impl BatchRecordsError {
    // the two model-discovery failures are the VertexAIError(400) LiteLLM raises; the rest are not provider responses
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::ModelMissing { .. } | Self::ProjectRequired { .. } => Some(400),
            Self::InputFileIdRequired
            | Self::IdentifierUtf8(_)
            | Self::NullJobState
            | Self::CreateTime(_)
            | Self::Decode(_)
            | Self::Usage(_) => None,
        }
    }
}
