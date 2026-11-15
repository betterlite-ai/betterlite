use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    Unavailable,
    Rejected,
}

#[derive(Debug, Error)]
#[error("Context compaction: {reason}")]
pub struct CompactionError {
    pub kind: ErrorKind,
    pub model: String,
    pub reason: String,
    #[source]
    pub source: Option<serde_json::Error>,
}
