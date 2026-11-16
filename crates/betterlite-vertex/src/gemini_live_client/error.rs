use thiserror::Error;

use crate::gemini_live_setup::error::LiveSetupError;

#[derive(Debug, Error)]
pub enum LiveClientError {
    #[error("the Gemini Live client event is larger than {limit} bytes")]
    TooLarge { limit: usize },
    #[error("Invalid JSON message: {message}")]
    InvalidJson {
        message: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("the Gemini Live client event is not a JSON object")]
    NotAnObject(#[source] serde_json::Error),
    #[error("`{path}` in the Gemini Live client event is not valid")]
    InvalidField {
        path: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("input_audio_buffer.append requires audio")]
    MissingAudio,
    #[error(transparent)]
    Setup(#[from] LiveSetupError),
    #[error("the Gemini Live client message could not be encoded")]
    Encode(#[source] serde_json::Error),
    #[error(transparent)]
    Sdk(#[from] super::sdk::SdkError),
}
