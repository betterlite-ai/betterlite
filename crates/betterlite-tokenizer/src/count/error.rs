use thiserror::Error;

use crate::engine::error::EngineError;
use crate::engine::fetch::FetchError;
use crate::engine::selection::SelectionError;
use crate::tools::error::ToolFormatError;

#[derive(Debug, Error)]
pub enum CountError {
    #[error("text and messages cannot both be set")]
    TextAndMessages,
    #[error("either text or messages must be provided")]
    MissingInput,
    #[error("tools or tool_choice cannot be set when counting text")]
    ToolsWithText,
    #[error("content block type {kind} is not counted")]
    UnsupportedContent { kind: String },
    #[error("the {field} field of a {kind} content block is not a string")]
    ContentField {
        kind: &'static str,
        field: &'static str,
    },
    #[error("the tokenizer could not be selected")]
    Selection(#[from] SelectionError),
    #[error("the tokenizer vocabulary could not be loaded")]
    Fetch(#[from] FetchError),
    #[error("the text could not be encoded")]
    Engine(#[from] EngineError),
    #[error("the tool definitions could not be formatted")]
    Tools(#[from] ToolFormatError),
    #[error("the request JSON could not be read or written")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the token count does not fit in a machine word")]
    Overflow,
    #[error("a text boundary does not fall on a character boundary")]
    Boundary,
    #[error("the exact-count limit is too large for an encode limit")]
    LimitsOverflow,
}
