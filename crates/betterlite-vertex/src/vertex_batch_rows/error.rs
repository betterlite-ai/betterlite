use std::str::Utf8Error;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum BatchRowsError {
    #[error("a vertex batch row is not usable JSON: {source}")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
    #[error("a vertex batch row is a JSON array, where an object is expected")]
    NotAnObject,
    #[error("the {field} of a vertex batch row is not UTF-8: {source}")]
    Utf8 {
        field: &'static str,
        #[source]
        source: Utf8Error,
    },
    #[error("the {label} label of a vertex batch row is not a string")]
    Label { label: String },
    #[error("the {field} of the batch entry {custom_id} is not the shape LiteLLM reads")]
    Shape {
        custom_id: String,
        field: &'static str,
    },
}
