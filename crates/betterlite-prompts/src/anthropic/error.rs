use std::error::Error as StdError;

use thiserror::Error;

use crate::anthropic::assembly::AssemblyError;
use crate::anthropic::assistant::AssistantError;
use crate::anthropic_user_blocks::error::UserBlocksError;
use crate::anthropic_user_blocks::prelude::PrepareError;

#[derive(Debug, Error)]
pub enum AnthropicPromptError<FileFailure>
where
    FileFailure: StdError + Send + Sync + 'static,
{
    #[error(transparent)]
    Prepare(#[from] PrepareError),
    #[error(transparent)]
    UserBlocks(#[from] UserBlocksError<FileFailure>),
    #[error(transparent)]
    Assistant(#[from] AssistantError),
    #[error(transparent)]
    Assembly(#[from] AssemblyError),
}
