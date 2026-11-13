use thiserror::Error;

use crate::api::responses_codex_tools::error::CustomToolsError;
use crate::api::responses_tool_namespaces::error::NamespaceToolsError;

#[derive(Debug, Error)]
pub enum StreamToolsError {
    #[error("the tool output index is past the largest supported value")]
    IndexOverflow,
    #[error("the event sequence number is past the largest supported value")]
    SequenceOverflow,
    #[error("the request tools must be a list")]
    ToolsNotList,
    #[error(transparent)]
    Custom(#[from] CustomToolsError),
    #[error(transparent)]
    Namespace(#[from] NamespaceToolsError),
    #[error("the web search call is not a response.web_search_call item")]
    WebSearch {
        #[source]
        source: serde_json::Error,
    },
}
