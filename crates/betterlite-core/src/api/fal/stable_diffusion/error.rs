use thiserror::Error;

use crate::api::fal::images::error::FalImageError;

#[derive(Debug, Error)]
pub enum StableDiffusionError {
    #[error("parameters {parameters:?} are not supported for model {model}")]
    UnsupportedParameters {
        parameters: Vec<String>,
        model: String,
    },
    #[error("the field size must be a string")]
    SizeNotAString,
    #[error("the url of image {index} in the reply is not a string")]
    InvalidImageUrl { index: usize },
    #[error(transparent)]
    Fal(#[from] FalImageError),
}

impl StableDiffusionError {
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::Fal(
                FalImageError::ReplyTooLarge { status, .. }
                | FalImageError::ReplyDecode { status, .. }
                | FalImageError::ReplyNotAnObject { status, .. },
            ) => Some(*status),
            Self::Fal(
                FalImageError::MissingApiKey | FalImageError::UnsupportedParameters { .. },
            )
            | Self::UnsupportedParameters { .. }
            | Self::SizeNotAString
            | Self::InvalidImageUrl { .. } => None,
        }
    }
}
