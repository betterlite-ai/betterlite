use betterlite_error::error::ErrorClass;
use thiserror::Error;

use crate::chat_content::ChatContentError;
use crate::sail_common::error::SailError;

#[derive(Debug, Error)]
pub enum SailChatError {
    #[error(transparent)]
    Sail(#[from] SailError),
    #[error(transparent)]
    Content(#[from] ChatContentError),
}

impl SailChatError {
    #[must_use]
    pub fn class(&self) -> Option<ErrorClass> {
        match self {
            Self::Sail(error) => Some(error.class()),
            Self::Content(_) => None,
        }
    }

    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::Sail(error) => error.status_code(),
            Self::Content(error) => error.status_code(),
        }
    }
}
