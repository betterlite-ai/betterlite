use thiserror::Error;

use super::call::CompletionCall;

// the shared HTTP completion handler the three platform functions hand their resolved call to
pub trait ChatCompletionHandler {
    type Reply;

    fn complete(&self, call: &CompletionCall) -> Result<Self::Reply, HandlerError>;
}

// the handler's own failure: the provider status when the handler has one, and the text it reports
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct HandlerError {
    pub status_code: Option<u16>,
    pub message: String,
}
