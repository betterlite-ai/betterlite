use betterlite_anthropic::anthropic_capability_catalog::sampling::SamplingError;
use betterlite_anthropic::anthropic_capability_catalog::thinking::ThinkingError;
use betterlite_anthropic::capabilities::reasoning::ReasoningEffortError;
use betterlite_anthropic::option_primitives::context_management::ContextManagementError;
use betterlite_anthropic::params::error::ParamsError;

use crate::bedrock_model_capabilities::CapabilityError;
use crate::converse_options::error::ConverseOptionsError;
use crate::converse_tools::error::ConverseToolsError;
use crate::error::BedrockError;

#[derive(Debug, thiserror::Error)]
pub enum ConverseParamsError {
    #[error(transparent)]
    Capability(#[from] CapabilityError),
    #[error(transparent)]
    Context(#[from] ContextManagementError),
    #[error(transparent)]
    Metadata(#[from] BedrockError),
    #[error(transparent)]
    Options(#[from] ConverseOptionsError),
    #[error(transparent)]
    Params(#[from] ParamsError),
    #[error(transparent)]
    Reasoning(#[from] ReasoningEffortError),
    #[error(transparent)]
    Sampling(#[from] SamplingError),
    #[error(transparent)]
    Thinking(#[from] ThinkingError),
    #[error(transparent)]
    Tools(#[from] ConverseToolsError),
    #[error("a parameter value cannot be encoded as JSON")]
    Encode(#[source] serde_json::Error),
    #[error("thinking.budget_tokens must be a number")]
    BudgetNotNumber,
    #[error("thinking.budget_tokens plus the default max tokens overflows")]
    BudgetOverflow,
}

impl ConverseParamsError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Capability(_) | Self::Context(_) | Self::Thinking(_) | Self::Encode(_) => None,
            Self::Metadata(error) => Some(error.status_code()),
            Self::Options(error) => error.status_code(),
            Self::Params(error) => error.status_code(),
            Self::Reasoning(_) | Self::BudgetNotNumber | Self::BudgetOverflow => Some(400),
            Self::Sampling(error) => error.status_code(),
            Self::Tools(error) => error.status_code(),
        }
    }
}
