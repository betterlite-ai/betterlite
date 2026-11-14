use thiserror::Error;

#[derive(Debug, Error)]
pub enum ResponsesTransformError {
    #[error("{field} must be a string")]
    NotText { field: &'static str },
    #[error("{field}: expected {expected}")]
    FieldType {
        field: &'static str,
        expected: &'static str,
    },
    #[error("{field} must be an object")]
    NotObject { field: &'static str },
    #[error("{field} must be a string, an object or a list of them")]
    NotInput { field: &'static str },
    #[error("the input content item must be text or an object")]
    NotContentItem,
    #[error("the interaction parameters could not be encoded: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("{source}")]
    Fields {
        #[source]
        source: serde_path_to_error::Error<serde_json::Error>,
    },
    #[error("the response creation time {seconds} is out of range")]
    CreatedAt {
        seconds: i64,
        #[source]
        source: jiff::Error,
    },
}
