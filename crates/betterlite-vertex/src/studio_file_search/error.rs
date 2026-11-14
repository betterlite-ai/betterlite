use thiserror::Error;

use super::ResponseHeaders;

#[derive(Debug, Error)]
pub enum StudioFileSearchError {
    #[error("GEMINI_API_KEY or GOOGLE_API_KEY is required")]
    MissingKey,
    #[error("Failed to parse Gemini response: {source}")]
    SearchReply {
        status: u16,
        headers: ResponseHeaders,
        #[source]
        source: serde_json::Error,
    },
    #[error("Failed to parse Gemini create response: {source}")]
    CreateReply {
        status: u16,
        headers: ResponseHeaders,
        #[source]
        source: serde_json::Error,
    },
    #[error("the File Search body cannot be represented as JSON")]
    Json {
        #[from]
        source: serde_json::Error,
    },
}
