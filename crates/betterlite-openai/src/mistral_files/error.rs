use betterlite_http::error::HttpError;
use betterlite_prompts::media_metadata::error::MediaMetadataError;
use thiserror::Error;

use crate::mistral_identity::error::MistralIdentityError;

use super::decode::MistralDecodeError;

#[derive(Debug, Error)]
pub enum MistralFilesError {
    #[error(transparent)]
    Identity(#[from] MistralIdentityError),
    #[error(
        "Mistral does not support purpose '{purpose}'; use one of: fine-tune, batch, ocr, user_data"
    )]
    UnsupportedPurpose { purpose: String },
    #[error("file_id is required to download Mistral file content")]
    FileIdRequired,
    #[error("the Mistral file URL cannot carry its file_id: {source}")]
    Path {
        #[source]
        source: HttpError,
    },
    #[error("the Mistral upload content cannot be read: {source}")]
    Content {
        #[source]
        source: MediaMetadataError,
    },
    #[error(transparent)]
    Decode(#[from] MistralDecodeError),
}

impl MistralFilesError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::UnsupportedPurpose { .. } => Some(400),
            Self::Identity(MistralIdentityError::Api { status, .. }) => Some(*status),
            Self::Identity(_)
            | Self::FileIdRequired
            | Self::Path { .. }
            | Self::Content { .. }
            | Self::Decode(_) => None,
        }
    }
}
