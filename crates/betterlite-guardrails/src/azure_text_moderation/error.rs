use reqwest::StatusCode;
use serde::Serialize;
use thiserror::Error;

use crate::azure_content_safety::error::AzureContentSafetyError;

#[derive(Debug, Error)]
pub enum AzureTextModerationError {
    #[error(transparent)]
    ContentSafety(#[from] AzureContentSafetyError),
    // The detail text of LiteLLM's HTTPException, down to its capitalisation.
    #[error(
        "Azure Content Safety Guardrail: {category} crossed severity {threshold}, Got severity: {severity}"
    )]
    Violation {
        category: String,
        threshold: i64,
        severity: i64,
    },
    #[error("the Azure Text Moderation text split into no chunks")]
    NoChunks,
    #[error("the Azure Text Moderation error frame could not be written as JSON")]
    Frame { source: serde_json::Error },
}

impl AzureTextModerationError {
    // LiteLLM raises HTTPException(400) for a violation and lets the transport's own status through otherwise.
    #[must_use]
    pub fn status_code(&self) -> StatusCode {
        match self {
            Self::Violation { .. } => StatusCode::BAD_REQUEST,
            Self::ContentSafety(source) => source.status_code(),
            Self::NoChunks | Self::Frame { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

// The dict LiteLLM puts in HTTPException.detail for a violation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ViolationDetail {
    pub error: String,
}
