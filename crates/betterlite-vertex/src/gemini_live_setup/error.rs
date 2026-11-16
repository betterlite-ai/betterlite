use thiserror::Error;

use crate::gemini_thinking::error::GeminiThinkingError;
use crate::gemini_tools::error::GeminiToolsError;

#[derive(Debug, Error)]
pub enum LiveSetupError {
    #[error("api_key is required for Gemini API calls")]
    MissingApiKey,
    #[error("the Gemini Live setup JSON is not valid")]
    InvalidJson(#[source] serde_json::Error),
    #[error("`{path}` in the Gemini Live setup JSON is not valid")]
    InvalidField {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("encoding the Gemini Live setup failed")]
    Encode(#[source] serde_json::Error),
    #[error(transparent)]
    Tools(#[from] GeminiToolsError),
    #[error(transparent)]
    Audio(#[from] GeminiThinkingError),
}
