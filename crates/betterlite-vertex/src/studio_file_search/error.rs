use thiserror::Error;

use super::ResponseHeaders;

#[derive(Debug, Error)]
pub enum StudioFileSearchError {
    #[error("GEMINI_API_KEY or GOOGLE_API_KEY is required")]
    MissingKey,
    #[error("Failed to parse Gemini response: {reason}")]
    SearchReply {
        status: u16,
        headers: ResponseHeaders,
        reason: String,
    },
    #[error("Failed to parse Gemini create response: {reason}")]
    CreateReply {
        status: u16,
        headers: ResponseHeaders,
        reason: String,
    },
    #[error("the reply value cannot be represented as JSON")]
    Json {
        #[from]
        source: serde_json::Error,
    },
}
