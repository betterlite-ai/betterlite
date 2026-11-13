use std::str::Utf8Error;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum BatchRowsError {
    #[error("a vertex batch row is not a JSON object: {source}")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the {field} of a vertex batch row is not UTF-8: {source}")]
    Utf8 {
        field: &'static str,
        #[source]
        source: Utf8Error,
    },
    #[error("the {label} label of a vertex batch row is not a string")]
    Label { label: String },
    #[error("the {field} of a vertex batch row is not the shape LiteLLM reads")]
    Shape { field: &'static str },
    #[error("the {label} label numbers a chunk past the 64-bit limit")]
    ChunkIndex { label: String },
    #[error("the embedding values of {custom_id} are not a JSON array of numbers")]
    Values { custom_id: String },
    #[error("the prompt tokens of {custom_id} do not fit a 32-bit embedding usage")]
    Tokens { custom_id: String },
    #[error("the batch output row of {custom_id} could not be encoded: {source}")]
    Encode {
        custom_id: String,
        #[source]
        source: serde_json::Error,
    },
}
