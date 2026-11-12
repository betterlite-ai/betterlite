use thiserror::Error;

use crate::api::responses_codex_tools::error::CustomToolsError;
use crate::api::responses_input_content::ContentError;
use crate::api::responses_input_messages::thinking::ThinkingError;
use crate::api::responses_input_messages::web_search::WebSearchError;
use crate::api::responses_reasoning_merge::MergeError;
use crate::api::responses_tool_results::error::ToolResultsError;

#[derive(Debug, Error)]
pub enum InputMessagesError {
    #[error("input item {position} is not a JSON object")]
    ItemNotObject { position: usize },
    #[error("the responses input must be a string or a list of input items")]
    InputShape,
    #[error("the responses input is not plain JSON")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Content(#[from] ContentError),
    #[error(transparent)]
    Role(#[from] CustomToolsError),
    #[error(transparent)]
    ToolResults(#[from] ToolResultsError),
    #[error(transparent)]
    Merge(#[from] MergeError),
    #[error(transparent)]
    Thinking(#[from] ThinkingError),
    #[error(transparent)]
    WebSearch(#[from] WebSearchError),
}

impl From<serde_json::Error> for InputMessagesError {
    fn from(source: serde_json::Error) -> Self {
        Self::Json { source }
    }
}
