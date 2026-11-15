use thiserror::Error;

use crate::gemini_response::decode::MAX_RESPONSE_BYTES;
use crate::model_urls::error::VertexUrlError;
use crate::vertex_recognition::error::VertexRecognitionError;

#[derive(Debug, Error)]
pub enum VertexGeminiTranscriptionError {
    #[error("the Vertex AI Gemini transcription request is not valid")]
    Shared {
        #[from]
        source: VertexRecognitionError,
    },
    #[error("the Vertex location is not valid: {source}")]
    Location {
        #[source]
        source: VertexUrlError,
    },
    #[error("vertex_project is required to build a Vertex AI Gemini transcription URL")]
    ProjectRequired,
    #[error("the Vertex AI Gemini transcription reply is not JSON")]
    NotJson {
        status: u16,
        #[source]
        source: serde_json::Error,
    },
    #[error("the Vertex AI Gemini transcription reply does not match the response at {path}")]
    Shape {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error(
        "the Vertex AI Gemini transcription reply is {size} bytes, over the {MAX_RESPONSE_BYTES} byte limit"
    )]
    TooLarge { size: usize },
    #[error("the Vertex AI Gemini transcription token counts do not fit the response")]
    CountOverflow,
}

impl VertexGeminiTranscriptionError {
    // the location, project and response_format failures are VertexAIError(400); the rest are not provider replies
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Shared { source } => source.status_code(),
            Self::Location { .. } | Self::ProjectRequired => Some(400),
            Self::NotJson { status, .. } => Some(*status),
            Self::Shape { .. } | Self::TooLarge { .. } | Self::CountOverflow => None,
        }
    }
}
