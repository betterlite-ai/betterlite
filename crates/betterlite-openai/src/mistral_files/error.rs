use super::decode::MistralDecodeError;
use crate::mistral_identity::error::MistralIdentityError;
use betterlite_http::error::HttpError;
use betterlite_prompts::media_metadata::error::MediaMetadataError;
use thiserror::Error;
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
    #[error("the Mistral file URL cannot carry its file_id: {0}")]
    Path(#[from] HttpError),
    #[error("the Mistral upload content cannot be read: {0}")]
    Content(#[from] MediaMetadataError),
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
            | Self::Path(_)
            | Self::Content(_)
            | Self::Decode(_) => None,
        }
    }
}
