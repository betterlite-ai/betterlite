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
    #[error("tool definition {index} is not a valid JSON object: {source}")]
    Object {
        index: usize,
        #[source]
        source: serde_json::Error,
    },
    #[error("tool definition {index} has an unsupported {field} shape: {source}")]
    Shape {
        index: usize,
        field: &'static str,
        #[source]
        source: serde_path_to_error::Error<serde_json::Error>,
    },
    #[error("{field} is missing from an array schema")]
    MissingItems { field: &'static str },
    #[error("{field} must be a JSON object")]
    NotAnObject { field: &'static str },
    #[error("{field} must be a JSON array")]
    NotArray { field: &'static str },
    #[error("{field} must be an array of strings")]
    NotTextArray { field: &'static str },
    #[error("{field} must hold scalar values")]
    NotScalar { field: &'static str },
    #[error("{field} must be a string")]
    NotText { field: &'static str },
}
