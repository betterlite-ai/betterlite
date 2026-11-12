use thiserror::Error;

use crate::gemini_response_parts::projection::PartsError;
use crate::gemini_usage::error::UsageError;

#[derive(Debug, Error)]
pub enum CandidatesError {
    #[error("a candidate part cannot be projected: {0}")]
    Parts(#[from] PartsError),
    #[error("the blocked response has no usable usage: {0}")]
    Usage(#[from] UsageError),
}
