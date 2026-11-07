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
    #[error("tool definition {index} has an unsupported shape: {source}")]
    Shape {
        index: usize,
        #[source]
        source: serde_path_to_error::Error<serde_json::Error>,
    },
    #[error("{field} is missing from an array schema")]
    MissingItems { field: &'static str },
}
