use betterlite_context::request_records::RequestRecordError;
use betterlite_prompts::bedrock_processor::error::ProcessorError;
use thiserror::Error;

use crate::bedrock_converse_tools_beta::error::ToolsAndBetaError;
use crate::bedrock_model_capabilities::CapabilityError;
use crate::converse_messages::error::ConverseMessageError;
use crate::converse_options::error::ConverseOptionsError;
use crate::converse_params::error::ConverseParamsError;
use crate::error::BedrockError;

#[derive(Debug, Error)]
pub enum ConverseRequestError {
    #[error(transparent)]
    Messages(#[from] ConverseMessageError),
    #[error(transparent)]
    Params(#[from] ConverseParamsError),
    #[error(transparent)]
    Tools(#[from] ToolsAndBetaError),
    #[error(transparent)]
    Capability(#[from] CapabilityError),
    #[error(transparent)]
    Options(#[from] ConverseOptionsError),
    #[error(transparent)]
    Processor(#[from] ProcessorError),
    #[error(transparent)]
    Metadata(#[from] BedrockError),
    #[error(transparent)]
    Records(#[from] RequestRecordError),
    #[error("{field} must be {expected}")]
    FieldType {
        field: &'static str,
        expected: &'static str,
    },
}

impl ConverseRequestError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Messages(error) => error.status_code(),
            Self::Params(error) => error.status_code(),
            Self::Tools(error) => error.status_code(),
            Self::Options(error) => error.status_code(),
            Self::Metadata(error) => Some(error.status_code()),
            Self::FieldType { .. } => Some(400),
            Self::Capability(_) | Self::Processor(_) | Self::Records(_) => None,
        }
    }
}
