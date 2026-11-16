use thiserror::Error;

use crate::dashscope_embedding::error::DashscopeError;

// the key failure is LiteLLM's ValueError, which carries no status; every reply failure is its BaseLLMException
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DashscopeImageError {
    #[error("{message}")]
    MissingKey { message: String },
    #[error("the DashScope image generation parameter `{name}` must be text")]
    ParamType { name: &'static str },
    #[error(transparent)]
    Reply(#[from] DashscopeError),
}
