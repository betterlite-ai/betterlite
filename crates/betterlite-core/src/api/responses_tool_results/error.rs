use thiserror::Error;

use crate::api::responses_codex_tools::error::CustomToolsError;

#[derive(Debug, Error)]
pub enum ToolResultsError {
    #[error("call_id must be a string or an integer")]
    CallId,
    #[error("{field} must be a string")]
    NotText { field: &'static str },
    #[error("{field} must be an object")]
    NotObject { field: &'static str },
    #[error("the cached tool call is not an object")]
    CachedNotObject,
    #[error("the cached tool call is not plain JSON")]
    CachedNotJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("provider_specific_fields has an invalid attribute")]
    Attributes {
        #[source]
        source: CustomToolsError,
    },
    #[error("the call id pattern does not compile")]
    Pattern {
        #[source]
        source: regex::Error,
    },
}
