use betterlite_bedrock::image_call::error::ImageCallError;
use betterlite_bedrock::image_edits::error::ImageEditError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BedrockImageError {
    #[error(transparent)]
    Generate(#[from] ImageCallError),
    #[error(transparent)]
    Edit(#[from] ImageEditError),
    #[error("Model needs to be set for bedrock")]
    ModelRequired,
    #[error("the supplied clock is before the Unix epoch")]
    Clock,
    #[error("the bedrock image composition runs only generation and edit operations")]
    UnsupportedOperation,
    #[error("betterlite does not run the {operation} operation through the Bedrock image provider")]
    Unsupported { operation: &'static str },
}

impl BedrockImageError {
    // The status the leaf's wrapped error carries; None is what a plain Python error reports
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::Generate(error) => error.status(),
            Self::Edit(error) => error.status(),
            Self::ModelRequired
            | Self::Clock
            | Self::UnsupportedOperation
            | Self::Unsupported { .. } => None,
        }
    }
}
