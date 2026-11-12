use betterlite_logging::analytics::python::FormatError;

use crate::api::response_chat_content::ContentError;
use crate::api::responses_tool_results::error::ToolResultsError;

#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    #[error(transparent)]
    Content(#[from] ContentError),
    #[error("tool call not supported: {tool_call}")]
    UnsupportedToolCall { tool_call: String },
    #[error("the tool call could not be printed the way Python prints it")]
    ToolCallPrint {
        #[source]
        source: FormatError,
    },
    #[error("a tool call is not an object with a string id")]
    ToolCallShape,
    #[error("the '{field}' field of a tool call is not a string")]
    ToolField { field: &'static str },
    #[error("provider_specific_fields could not be read")]
    ProviderFields {
        #[source]
        source: serde_json::Error,
    },
    #[error("the call id pattern does not compile")]
    CallId {
        #[source]
        source: ToolResultsError,
    },
    #[error("a thinking block is not a content block")]
    ThinkingBlock {
        #[source]
        source: serde_json::Error,
    },
    #[error("reasoning item {index} has an invalid '{field}' field")]
    ReasoningField { index: usize, field: &'static str },
    #[error("reasoning item {index} carries a summary that is not a list of blocks")]
    SummaryShape { index: usize },
    #[error("summary block {block} of reasoning item {index} has an invalid '{field}' field")]
    SummaryBlockField {
        index: usize,
        block: usize,
        field: &'static str,
    },
    #[error("a reasoning item could not be serialized")]
    Serialize {
        #[source]
        source: serde_json::Error,
    },
}
