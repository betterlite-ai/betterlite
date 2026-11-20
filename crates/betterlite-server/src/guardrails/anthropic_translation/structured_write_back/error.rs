use thiserror::Error;

use betterlite_core::api::messages_chat_transform::error::TransformError;
use betterlite_prompts::anthropic::error::AnthropicPromptError;
use betterlite_prompts::anthropic_invokes::file_blocks::FileBlockError;

#[derive(Debug, Error)]
pub enum StructuredWriteBackError {
    #[error(transparent)]
    Translation(#[from] TransformError),
    #[error(transparent)]
    Conversion(#[from] AnthropicPromptError<FileBlockError>),
    #[error("the structured messages could not be carried as the prompts crate's JSON")]
    Encoding(#[from] serde_json::Error),
    #[error("a tool exchange group names a row the run does not have")]
    RowMissing,
}
