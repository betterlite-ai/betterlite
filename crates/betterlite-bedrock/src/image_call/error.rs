use betterlite_logging::logging_pre_post::error::PrePostError;
use thiserror::Error;

use super::decode::ReplyDecodeError;
use super::transport::TransferError;
use crate::aws_endpoints::error::AwsEndpointError;
use crate::bedrock_settings::{BedrockError, HeaderError};
use crate::image_models::error::ImageModelsError;
use crate::signing::error::SigningError;

#[derive(Debug, Error)]
pub enum ImageCallError {
    #[error(transparent)]
    Models(#[from] ImageModelsError),
    #[error(transparent)]
    Signing(#[from] SigningError),
    #[error(transparent)]
    Endpoint(#[from] AwsEndpointError),
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
    Decode(#[from] ReplyDecodeError),
    #[error(transparent)]
    Reply(#[from] serde_json::Error),
    #[error(transparent)]
    Transport(#[from] TransferError),
    #[error("guardrail {0} must be text")]
    GuardrailType(&'static str),
}

impl ImageCallError {
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::Provider { status, .. }
            | Self::Models(ImageModelsError::Provider { status, .. }) => Some(*status),
            Self::Models(
                ImageModelsError::Size(_)
                | ImageModelsError::KeywordCollision(_)
                | ImageModelsError::TaskTypeUnsupported(_)
                | ImageModelsError::NotAMapping(_)
                | ImageModelsError::JoinNotAString(..)
                | ImageModelsError::Header(_),
            )
            | Self::Signing(_)
            | Self::Endpoint(_)
            | Self::Header(_)
            | Self::Logging(_)
            | Self::Decode(_)
            | Self::Reply(_)
            | Self::Transport(_)
            | Self::GuardrailType(_) => None,
        }
    }
}
