use betterlite_anthropic::feature_flags::FeatureFlagsError;
use betterlite_prompts::bedrock_tools::error::BedrockToolError;

use crate::converse_tools::error::ConverseToolsError;

#[derive(Debug, thiserror::Error)]
pub enum ToolsAndBetaError {
    #[error(transparent)]
    Tools(#[from] ConverseToolsError),
    #[error(transparent)]
    Tool(#[from] BedrockToolError),
    #[error(transparent)]
    Features(#[from] FeatureFlagsError),
    #[error("the tool tree does not convert: {0}")]
    Json(#[from] serde_json::Error),
}

impl ToolsAndBetaError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Tools(error) => error.status_code(),
            Self::Features(error) => error.status_code(),
            Self::Tool(_) | Self::Json(_) => None,
        }
    }
}
