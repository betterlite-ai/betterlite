use thiserror::Error;

// LiteLLM raises this when a batch fails in a way a later flush could still deliver
#[derive(Debug, Error)]
#[error("{detail}")]
pub struct PointFiveUploadError {
    pub detail: String,
}
