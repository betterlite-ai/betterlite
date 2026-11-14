use betterlite_logging::logging_pre_post::error::PrePostError;
use thiserror::Error;

use super::decode::RerankDecodeError;
use super::transport::TransferError;
use crate::bedrock_settings::{BedrockError, HeaderError};
use crate::signing::error::SigningError;

#[derive(Debug, Error)]
pub enum RerankCallError {
    #[error(transparent)]
    Signing(#[from] SigningError),
    #[error("bedrock error ({status})")]
    Provider {
        status: u16,
        error: Box<BedrockError>,
    },
    #[error(transparent)]
    Header(#[from] HeaderError),
    #[error(transparent)]
    Logging(#[from] PrePostError),
    #[error(transparent)]
    Decode(#[from] RerankDecodeError),
    #[error(transparent)]
    Transport(#[from] TransferError),
}

impl RerankCallError {
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::Provider { status, .. } => Some(*status),
            Self::Signing(_)
            | Self::Header(_)
            | Self::Logging(_)
            | Self::Decode(_)
            | Self::Transport(_) => None,
        }
    }
}
