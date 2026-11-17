use thiserror::Error;

use super::call::CompletionCall;

// the provider config call main.py's self-hosted arms make, with the values each arm resolved
pub trait ChatCompletionHandler {
    type Reply;

    fn complete(&self, call: &CompletionCall) -> Result<Self::Reply, HandlerError>;
}

#[derive(Debug, Error)]
pub enum HandlerError {
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
            Self::Upstream { status_code } => Some(*status_code),
            Self::EmptyReply => None,
        }
    }
}
