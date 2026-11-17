use betterlite_anthropic::capabilities::effort::EffortError;
use betterlite_anthropic::capabilities::reasoning::ReasoningEffortError;

use crate::bedrock_model_capabilities::CapabilityError;

pub const BAD_REQUEST_STATUS: u16 = 400;
pub const BEDROCK_CONVERSE_PROVIDER: &str = "bedrock_converse";
pub const NOVA_2_REJECTION_MODEL: &str = "amazon.nova-2-lite-v1:0";

#[derive(Debug, thiserror::Error)]
pub enum ConverseOptionsError {
    #[error(
        "Invalid reasoning_effort value '{value}' for Nova 2 models. Supported values: ['low', 'medium', 'high']"
    )]
    NovaEffort {
        value: String,
        model: String,
        provider: String,
    },
    #[error(transparent)]
    UnmappedEffort(#[from] ReasoningEffortError),
    #[error(transparent)]
    InvalidEffort(#[from] EffortError),
    #[error(
        "Invalid reasoning_effort/output_config.effort value: {label}. Must be one of: 'low', 'medium', 'high', 'xhigh', or 'max'."
    )]
    AdaptiveEffort {
        label: String,
        model: String,
        provider: String,
    },
    #[error("{message}")]
    UnsupportedEffort {
        message: String,
        model: String,
        provider: String,
    },
    #[error(transparent)]
    Capability(#[from] CapabilityError),
}

impl ConverseOptionsError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Capability(_) => None,
            Self::NovaEffort { .. }
            | Self::UnmappedEffort(_)
            | Self::InvalidEffort(_)
            | Self::AdaptiveEffort { .. }
            | Self::UnsupportedEffort { .. } => Some(BAD_REQUEST_STATUS),
        }
    }
}
