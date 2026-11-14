use thiserror::Error;

#[derive(Debug, Error)]
pub enum AgenticHookError {
    #[error("callback {callback} failed: {message}")]
    Failed { callback: String, message: String },
}
