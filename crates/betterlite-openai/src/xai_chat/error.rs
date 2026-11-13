use thiserror::Error;

use crate::chat_content::ChatContentError;
use crate::xai_tokens::error::XaiOauthError;
use crate::xai_usage::error::XaiUsageError;

#[derive(Debug, Error)]
pub enum XaiChatError {
    #[error("the xAI OAuth token for model {model} could not be obtained")]
    Authentication {
        model: String,
        #[source]
        source: XaiOauthError,
    },
    #[error(transparent)]
    Content(#[from] ChatContentError),
    #[error(transparent)]
    Usage(#[from] XaiUsageError),
    #[error("the tools value is not a list")]
    ToolsNotList,
    #[error("a tool entry is not an object")]
    ToolNotObject,
    #[error("the xAI reply is not a chat completion")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
}

impl XaiChatError {
    // LiteLLM raises AuthenticationError with status 401, and a malformed tools value is bad input
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::Authentication { .. } => 401,
            Self::Content(source) => source.status_code(),
            Self::ToolsNotList | Self::ToolNotObject => 400,
            Self::Usage(_) | Self::Decode { .. } => 500,
        }
    }
}
