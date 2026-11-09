use thiserror::Error;

// LiteLLM's JinaAIError carries a status and message only, so get_error_class drops the headers
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum JinaAiError {
    #[error("Jina AI answered with status {status_code}: {message}")]
    Provider { status_code: u16, message: String },
}
