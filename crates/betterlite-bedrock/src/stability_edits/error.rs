use betterlite_models::image_cost::error::ImageCostError;
use thiserror::Error;

use crate::bedrock_settings::{BedrockError, HeaderError};

#[derive(Debug, Error)]
pub enum StabilityReplyError {
    #[error("the response body is not JSON: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the response body is not a JSON object")]
    NotAnObject,
    #[error("the {field} value has a type the Stability reply does not declare")]
    Field { field: String },
}

#[derive(Debug, Error)]
pub enum StabilityEditError {
    #[error(
        "Parameter {parameter} is not supported for model {model}. Supported parameters are ['n', 'size', 'response_format', 'mask']. Set drop_params=True to drop unsupported parameters."
    )]
    UnsupportedParameter { parameter: String, model: String },
    #[error("the {field} value has a type the Stability transform does not declare")]
    Field { field: String },
    #[error("Bedrock has no image price entry for {model}")]
    ModelInfo { model: String },
    #[error(transparent)]
    Price(#[from] ImageCostError),
    #[error(transparent)]
    Reply(#[from] StabilityReplyError),
    #[error("bedrock error ({status})")]
    Provider {
        status: u16,
        error: Box<BedrockError>,
    },
    #[error(transparent)]
    Header(#[from] HeaderError),
}

impl StabilityEditError {
    // The status LiteLLM's BedrockError carries; None is what a Python ValueError or TypeError reports
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::Provider { status, .. } => Some(*status),
            Self::UnsupportedParameter { .. }
            | Self::Field { .. }
            | Self::ModelInfo { .. }
            | Self::Price(_)
            | Self::Reply(_)
            | Self::Header(_) => None,
        }
    }
}
