use betterlite_tokenizer::count::error::CountError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ClearToolUsesError {
    #[error("context_management token counting failed")]
    TokenCount(#[source] CountError),
    #[error("a synchronous token count ran inside an async runtime")]
    BlockingRuntime,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct ContextManagementError {
    pub status_code: u16,
    pub message: String,
}
