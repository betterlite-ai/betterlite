use thiserror::Error;

use crate::a2a_chat::error::{A2aError, A2aFailure};
use crate::langflow_chat::error::LangFlowError;
use crate::langgraph_chat::error::{LangGraphError, LangGraphFailure};

use super::call::CompletionCall;

pub trait ChatCompletionHandler {
    type Reply;

    fn complete(&self, call: &CompletionCall) -> Result<Self::Reply, HandlerError>;
}

#[derive(Debug, Error)]
pub enum HandlerError {
    #[error(transparent)]
    A2a(#[from] A2aFailure),
    #[error(transparent)]
    A2aReply(#[from] A2aError),
    #[error(transparent)]
    LangGraph(#[from] LangGraphFailure),
    #[error(transparent)]
    LangGraphReply(#[from] LangGraphError),
    #[error(transparent)]
    LangFlow(#[from] LangFlowError),
    // the provider answered an error status, before any reply transform runs
    #[error("the provider answered with status {status_code}")]
    Upstream { status_code: u16 },
    #[error("the provider reply holds no choice")]
    EmptyReply,
}

impl HandlerError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::A2a(_) | Self::LangGraph(_) | Self::EmptyReply => None,
            Self::A2aReply(error) => Some(error.status_code),
            Self::LangGraphReply(error) => Some(error.status_code),
            Self::LangFlow(error) => error.status_code(),
            Self::Upstream { status_code } => Some(*status_code),
        }
    }
}
