use betterlite_prompts::gemini_tool_invokes::error::GeminiToolInvokesError;
use betterlite_prompts::gemini_tool_results::error::GeminiToolResultsError;
use thiserror::Error;

use crate::gemini_media::error::MediaError;

#[derive(Debug, Error)]
pub enum HistoryError {
    #[error("{0}")]
    Invalid(String),
    #[error("the history has more than {0} messages")]
    TooManyMessages(usize),
    #[error("a message has more than {0} content parts")]
    TooManyParts(usize),
    #[error("a message field is not the declared type")]
    Decode(#[source] serde_json::Error),
    #[error("{0}")]
    Media(#[from] MediaError),
    #[error("{0}")]
    ToolInvoke(#[from] GeminiToolInvokesError),
    #[error("{0}")]
    ToolResult(#[from] GeminiToolResultsError),
    #[error(
        "Unable to determine mime type for file: {named}, set this explicitly using file.format (or file.mime_type/content_type). Original error: {source}"
    )]
    FileMedia { named: String, source: MediaError },
    #[error("a content part cannot be encoded")]
    Encode(#[source] serde_json::Error),
}

impl HistoryError {
    #[must_use]
    pub const fn status(&self) -> u16 {
        if let Self::Media(source) = self {
            return source.status();
        }
        400
    }

    // every error except a media one is a bodyless BadRequestError: type and param null, code 400
    #[must_use]
    pub const fn error_type(&self) -> Option<&'static str> {
        if let Self::Media(source) = self {
            return source.error_type();
        }
        None
    }

    #[must_use]
    pub const fn param(&self) -> Option<&'static str> {
        if let Self::Media(source) = self {
            return source.param();
        }
        None
    }

    #[must_use]
    pub const fn code(&self) -> Option<&'static str> {
        if let Self::Media(source) = self {
            return source.code();
        }
        Some("400")
    }
}
