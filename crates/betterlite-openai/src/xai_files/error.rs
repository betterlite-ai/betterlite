use betterlite_http::error::HttpError;
use betterlite_prompts::media_metadata::error::MediaMetadataError;
use thiserror::Error;

use crate::xai_batches::error::XaiBatchesError;

#[derive(Debug, Error)]
pub enum XaiFilesError {
    #[error("File data is required")]
    FileDataRequired,
    #[error(transparent)]
    Media(#[from] MediaMetadataError),
    #[error(transparent)]
    Shared(#[from] XaiBatchesError),
    #[error("the xAI file id cannot be put in a URL path: {source}")]
    Path {
        #[source]
        source: HttpError,
    },
    #[error("the xAI file answer is not the expected JSON: {source}")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the xAI file answer exceeds the {limit}-byte limit")]
    TooLarge { limit: usize },
}

impl XaiFilesError {
    // LiteLLM's ValueError carries no status, its BaseLLMException carries one
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Shared(error) => error.status_code(),
            Self::FileDataRequired
            | Self::Media(_)
            | Self::Path { .. }
            | Self::Decode { .. }
            | Self::TooLarge { .. } => None,
        }
    }
}
