use thiserror::Error;

#[derive(Debug, Error)]
pub enum NativeCompactionError {
    #[error("compaction response field {field} must be an object")]
    NotAnObject { field: &'static str },
    #[error("compaction response field {field} must be a list of objects")]
    NotAnObjectList { field: &'static str },
    #[error("compaction payload has no {field}")]
    Missing { field: &'static str },
    #[error("compaction payload field {field} must be a string")]
    NotText { field: &'static str },
    #[error("compaction payload field {field} must be a list")]
    NotAList { field: &'static str },
    #[error("compaction payload message {index} must be an object with a chat role")]
    BadMessage { index: usize },
}
