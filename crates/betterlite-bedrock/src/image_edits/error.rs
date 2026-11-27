use betterlite_logging::logging_pre_post::error::PrePostError;
use thiserror::Error;

use crate::aws_endpoints::error::AwsEndpointError;
use crate::bedrock_settings::{BedrockError, HeaderError};
use crate::canvas_edits::error::CanvasEditError;
use crate::canvas_results::CanvasResultError;
use crate::image_call::transport::TransferError;
use crate::signing::error::SigningError;
use crate::stability_edits::error::StabilityEditError;

#[derive(Debug, Error)]
pub enum ImageEditError {
    #[error(transparent)]
    Canvas(#[from] CanvasEditError),
    #[error(transparent)]
    CanvasReply(#[from] CanvasResultError),
    #[error(transparent)]
    Stability(#[from] StabilityEditError),
    #[error(transparent)]
    Signing(#[from] SigningError),
    #[error(transparent)]
    Endpoint(#[from] AwsEndpointError),
    #[error(transparent)]
    Header(#[from] HeaderError),
    #[error(transparent)]
    Logging(#[from] PrePostError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("Error in response object format, got None")]
    NullReply,
    #[error("the edit request body is not a JSON object")]
    BodyShape,
    #[error("bedrock error ({status})")]
    Provider {
        status: u16,
        error: Box<BedrockError>,
    },
    #[error(transparent)]
    Transport(#[from] TransferError),
}

impl ImageEditError {
    // The status LiteLLM's wrapped exception carries; None is what a plain Python error reports
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::Canvas(error) => error.status(),
            Self::CanvasReply(error) => error.status(),
            Self::Stability(error) => error.status(),
            Self::Provider { status, .. } => Some(*status),
            Self::Signing(_)
            | Self::Endpoint(_)
            | Self::Header(_)
            | Self::Logging(_)
            | Self::Json(_)
            | Self::NullReply
            | Self::BodyShape
            | Self::Transport(_) => None,
        }
    }
}
