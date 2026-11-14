use thiserror::Error;

use crate::gemini_response::decode::MAX_RESPONSE_BYTES;

#[derive(Debug, Error)]
pub enum StudioTranscriptionError {
    #[error(
        "Google API key is required. Set GOOGLE_API_KEY or GEMINI_API_KEY environment variable."
    )]
    MissingApiKey,
    #[error(
        "the Gemini Interactions reply is {size} bytes, over the {MAX_RESPONSE_BYTES} byte limit"
    )]
    TooLarge { size: usize },
    #[error("the Gemini Interactions reply is not JSON")]
    NotJson {
        status: u16,
        #[source]
        source: serde_json::Error,
    },
    #[error("the Gemini Interactions reply does not match the transcription response at {path}")]
    Shape {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("Gemini transcription interaction did not complete (status={status:?})")]
    Incomplete {
        status: Option<String>,
        upstream: u16,
    },
    #[error("the Studio transcription usage token counts overflow")]
    UsageOverflow,
    #[error("the Studio transcription request or record cannot be written as JSON")]
    Json {
        #[source]
        source: serde_json::Error,
    },
}

impl StudioTranscriptionError {
    // the missing key and the reply failures are GeminiError with that status; a shape failure is pydantic's ValidationError
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::MissingApiKey => Some(401),
            Self::NotJson { status, .. }
            | Self::Incomplete {
                upstream: status, ..
            } => Some(*status),
            Self::TooLarge { .. }
            | Self::Shape { .. }
            | Self::UsageOverflow
            | Self::Json { .. } => None,
        }
    }
}
