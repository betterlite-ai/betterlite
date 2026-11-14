use crate::gemini_candidates::error::CandidatesError;
use crate::gemini_response::error::GenerateContentError;
use crate::gemini_usage::error::UsageError;
use thiserror::Error;
#[derive(Debug, Error)]
pub enum StreamError {
    #[error("the Gemini stream fragment is not UTF-8: {source}")]
    Utf8 {
        #[source]
        source: std::str::Utf8Error,
    },
    #[error("the Gemini stream chunk is not a JSON object")]
    NotAnObject,
    #[error("the retained Gemini stream buffer is {size} bytes, over the limit")]
    TooLarge { size: usize },
    #[error("the Gemini stream chunk is malformed: {0}")]
    Decode(#[from] GenerateContentError),
    #[error("the Gemini stream chunk cannot be projected: {0}")]
    Candidates(#[from] CandidatesError),
    #[error("the Gemini stream usage cannot be counted: {0}")]
    Usage(#[from] UsageError),
    #[error("the Gemini stream chunk reports an error: {message}")]
    Embedded { status: u16, message: String },
    #[error("the Gemini stream transport did not close cleanly")]
    Close,
}
