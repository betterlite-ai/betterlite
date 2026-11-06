use thiserror::Error;

#[derive(Debug, Error)]
pub enum ToolFormatError {
    #[error("the tool definitions are {bytes} bytes, over the {limit} byte limit")]
    TooLarge { bytes: usize, limit: usize },
    #[error("the tool definitions hold {count} entries, over the {limit} entry limit")]
    TooMany { count: usize, limit: usize },
    #[error("the tool definitions are not a JSON array: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("{field} must be a JSON object")]
    NotAnObject { field: &'static str },
    #[error("{field} must be a JSON array")]
    NotAnArray { field: &'static str },
    #[error("{field} must be a string")]
    NotText { field: &'static str },
    #[error("{field} must be an array of strings")]
    NotTextArray { field: &'static str },
    #[error("{field} must hold scalar values")]
    NotScalar { field: &'static str },
    #[error("{field} is missing from an array schema")]
    MissingItems { field: &'static str },
}
