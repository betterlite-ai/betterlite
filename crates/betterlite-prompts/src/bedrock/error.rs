use thiserror::Error;

use crate::bedrock_processor::error::ProcessorError;
use crate::bedrock_tools::error::BedrockToolError;

#[derive(Debug, Error)]
pub enum PromptError {
    #[error(transparent)]
    Processor(#[from] ProcessorError),
    #[error(transparent)]
    Tools(#[from] BedrockToolError),
}
