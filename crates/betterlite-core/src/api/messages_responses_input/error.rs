use crate::api::responses_usage::ResponsesUsageError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MessagesResponsesInputError {
    #[error("failed to encode the input of tool {tool:?} as JSON: {source}")]
    ToolInput {
        tool: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("responses usage translation failed: {source}")]
    Usage {
        #[source]
        source: ResponsesUsageError,
    },
    #[error("translated usage is not a chat usage record: {source}")]
    UsageRecord {
        #[source]
        source: serde_json::Error,
    },
}
