use std::str::Utf8Error;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum BatchRowsError {
    #[error("a vertex batch row is not usable JSON: {0}")]
    Decode(#[from] serde_json::Error),
    #[error("a vertex batch row is a JSON array, where an object is expected")]
    NotAnObject,
    #[error("the {0} of a vertex batch row is not UTF-8: {1}")]
    Utf8(&'static str, #[source] Utf8Error),
    #[error("the {0} label of a vertex batch row is not a string")]
    Label(String),
    #[error("the {1} of the batch entry {0} is not the shape LiteLLM reads")]
    Shape(String, &'static str),
}
