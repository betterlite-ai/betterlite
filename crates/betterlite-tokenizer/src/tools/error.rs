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
    #[error("tool definition {index} has an unsupported {field} shape: {source}")]
    Shape {
        index: usize,
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("{field} does not match the expected shape: {source}")]
    Nested {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("the tool schema nests deeper than {limit} levels")]
    TooDeep { limit: usize },
    #[error("{field} is missing from an array schema")]
    MissingItems { field: &'static str },
    #[error("{field} must hold scalar values")]
    NotScalar { field: &'static str },
}
