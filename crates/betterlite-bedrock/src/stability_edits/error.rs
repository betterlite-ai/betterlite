use betterlite_models::image_cost::error::ImageCostError;
use thiserror::Error;

use crate::bedrock_settings::{BedrockError, ErrorHeaders, ErrorInput, HeaderError};

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

// B682 BedrockError.__init__, which keeps the message, the status and the headers as given
#[must_use]
pub(super) fn provider_error(
    status_code: u16,
    message: String,
    headers: Option<ErrorHeaders>,
) -> StabilityEditError {
    let input = ErrorInput {
        status_code,
        message,
        request: None,
        body: None,
        status_code_is_synthesized: false,
        headers,
        response: None,
    };
    match BedrockError::try_from(input) {
        Ok(error) => StabilityEditError::Provider {
            status: status_code,
            error: Box::new(error),
        },
        Err(source) => StabilityEditError::Header(source),
    }
}
