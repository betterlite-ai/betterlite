use betterlite_prompts::gemini_tool_invokes::error::GeminiToolInvokesError;
use betterlite_prompts::gemini_tool_results::error::GeminiToolResultsError;
use thiserror::Error;

use crate::gemini_media::error::MediaError;

#[derive(Debug, Error)]
pub enum HistoryError {
    #[error("the history has more than {limit} messages")]
    TooManyMessages { limit: usize },
    #[error("message {index} has more than {limit} content parts")]
    TooManyParts { index: usize, limit: usize },
    #[error("message {index} has role {role}, which a Gemini history cannot carry")]
    UnsupportedRole { index: usize, role: String },
    #[error("field {path} is missing")]
    MissingField { path: String },
    #[error("field {path} must be {expected}")]
    WrongField {
        path: String,
        expected: &'static str,
    },
    #[error("field {path} must be a video metadata object")]
    VideoMetadata {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("{source}")]
    Media {
        #[from]
        source: MediaError,
    },
    #[error("{source}")]
    ToolInvoke {
        #[from]
        source: GeminiToolInvokesError,
    },
    #[error("{source}")]
    ToolResult {
        #[from]
        source: GeminiToolResultsError,
    },
    #[error("the tool calls of a previous assistant message cannot be read")]
    ToolCalls {
        #[source]
        source: serde_json::Error,
    },
    #[error("a content part did not encode as a JSON object")]
    PartShape,
    #[error("a content part cannot be encoded")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}

impl HistoryError {
    #[must_use]
    pub const fn status(&self) -> u16 {
        if let Self::Media { source } = self {
            return source.status();
        }
        400
    }

    #[must_use]
    pub const fn error_type(&self) -> Option<&'static str> {
        if let Self::Media { source } = self {
            return source.error_type();
        }
        None
    }

    #[must_use]
    pub const fn param(&self) -> Option<&'static str> {
        if let Self::Media { source } = self {
            return source.param();
        }
        None
    }

    #[must_use]
    pub const fn code(&self) -> Option<&'static str> {
        if let Self::Media { source } = self {
            return source.code();
        }
        Some("400")
    }
}
