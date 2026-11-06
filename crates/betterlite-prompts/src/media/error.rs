use base64::DecodeError;
use thiserror::Error;

use crate::media_fetch::error::ImageFetchError;

// One enum for the module, so parallel tasks never append to a shared one
#[derive(Debug, Error)]
pub enum MediaError {
    #[error("the data URL does not start with data:")]
    MissingDataPrefix,
    #[error("the data URL has no ;base64, marker")]
    MissingBase64Marker,
    #[error("the data URL decodes to more than the {limit} bytes this call allows")]
    TooLarge { limit: usize },
    #[error("the base64 payload is invalid: {0}")]
    InvalidBase64(#[from] DecodeError),
    #[error(transparent)]
    Fetch(#[from] ImageFetchError),
}
