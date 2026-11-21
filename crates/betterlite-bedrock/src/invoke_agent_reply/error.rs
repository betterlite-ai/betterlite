use thiserror::Error;

use crate::bedrock_settings::{BedrockError, HeaderError};
use crate::event_frames::error::EventFrameError;
use crate::invoke_agent_reply::extract::ExtractError;

// B682's BedrockError implements no Display or Error, so the derived wrapper carries the cause instead.
#[derive(Debug, Error)]
#[error("bedrock provider error")]
pub struct BedrockFailure {
    pub cause: Box<BedrockError>,
}

#[derive(Debug, Error)]
pub enum InvokeAgentReplyError {
    #[error(transparent)]
    Frame(#[from] EventFrameError),
    #[error(transparent)]
    Extract(#[from] ExtractError),
    #[error("bedrock provider error ({status})")]
    Bedrock {
        status: u16,
        #[source]
        error: BedrockFailure,
    },
    #[error(transparent)]
    Header(#[from] HeaderError),
}
