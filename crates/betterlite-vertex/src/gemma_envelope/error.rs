use betterlite_openai::chat_content::ChatContentError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GemmaEnvelopeError {
    #[error("the Vertex Gemma reply has no 'predictions' field")]
    MissingPredictions,
    #[error("{message}")]
    Container { code: u16, message: String },
    #[error(transparent)]
    Shape(#[from] ChatContentError),
}

impl GemmaEnvelopeError {
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::MissingPredictions => 422,
            Self::Container { code, .. } => *code,
            Self::Shape(source) => source.status_code(),
        }
    }
}
