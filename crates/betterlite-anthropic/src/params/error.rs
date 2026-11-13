use betterlite_openai::error::OpenAiError;

use crate::anthropic_capability_catalog::forced_tool::ForcedToolChoiceError;
use crate::anthropic_capability_catalog::sampling::SamplingError;
use crate::anthropic_capability_catalog::thinking::ThinkingError;
use crate::capabilities::effort::EffortError;
use crate::capabilities::reasoning::ReasoningEffortError;
use crate::capabilities::speed::SpeedError;
use crate::option_primitives::OptionPrimitivesError;
use crate::option_primitives::context_management::ContextManagementError;
use crate::option_primitives::user_id::UserIdError;
use crate::output_schema::SchemaError;
use crate::tools::ToolsError;

#[derive(Debug, thiserror::Error)]
pub enum ParamsError {
    #[error(transparent)]
    Base(#[from] OpenAiError),
    #[error(transparent)]
    Context(#[from] ContextManagementError),
    #[error(transparent)]
    Effort(#[from] EffortError),
    #[error(transparent)]
    ForcedTool(#[from] ForcedToolChoiceError),
    #[error(transparent)]
    Options(#[from] OptionPrimitivesError),
    #[error(transparent)]
    Reasoning(#[from] ReasoningEffortError),
    #[error(transparent)]
    Sampling(#[from] SamplingError),
    #[error(transparent)]
    Schema(#[from] SchemaError),
    #[error(transparent)]
    Speed(#[from] SpeedError),
    #[error(transparent)]
    Thinking(#[from] ThinkingError),
    #[error(transparent)]
    Tools(#[from] ToolsError),
    #[error(transparent)]
    UserId(#[from] UserIdError),
    #[error("{field} must be a number")]
    NotNumber { field: &'static str },
    #[error("{field} must be a list")]
    NotList { field: &'static str },
    #[error("a parameter value is not an object")]
    NotAnObject,
    #[error("{field} does not fit a token count")]
    OutOfRange { field: &'static str },
    #[error("{field} must be a boolean")]
    NotBool { field: &'static str },
    #[error(
        "Invalid effort value: {value}. Must be one of: 'high', 'medium', 'low', 'xhigh', 'max'."
    )]
    InvalidEffort { value: String },
    #[error("{message}")]
    EffortUnsupported { message: String },
    #[error("a parameter value cannot be encoded as JSON")]
    Encode(#[source] serde_json::Error),
    #[error("a parameter value cannot be read as JSON")]
    Decode(#[source] serde_json::Error),
}

impl ParamsError {
    #[must_use]
    pub fn status_code(&self) -> Option<u16> {
        match self {
            Self::Sampling(error) => error.status_code(),
            Self::Speed(error) => error.status_code(),
            Self::ForcedTool(error) => error.status_code(),
            Self::Tools(error) => error.status_code(),
            Self::Effort(_)
            | Self::Reasoning(_)
            | Self::InvalidEffort { .. }
            | Self::EffortUnsupported { .. }
            | Self::NotNumber { .. }
            | Self::NotList { .. }
            | Self::NotAnObject { .. }
            | Self::OutOfRange { .. }
            | Self::NotBool { .. } => Some(400),
            Self::Base(_)
            | Self::Context(_)
            | Self::Options(_)
            | Self::Schema(_)
            | Self::Thinking(_)
            | Self::UserId(_)
            | Self::Encode(_)
            | Self::Decode(_) => None,
        }
    }
}
