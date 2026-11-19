use thiserror::Error;

use super::call::CompletionCall;

// LiteLLM's base_llm_http_handler.completion, which the arm calls with its resolved values
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
