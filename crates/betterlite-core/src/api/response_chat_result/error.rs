use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

use crate::api::response_chat_history::error::HistoryError;
use crate::api::responses_sse_recovery::ResponsesSseRecoveryError;
use crate::api::responses_tool_results::error::ToolResultsError;
use crate::api::responses_usage::ResponsesUsageError;

#[derive(Debug, Error)]
pub enum ResponseChatResultError {
    #[error("Error in response: {body}")]
    ResponseError { body: String },
    #[error("Unknown items in responses API response: {items}")]
    UnknownItems { items: String },
    #[error("the {field} field of a raw output item is not a value the chat field accepts")]
    RawField { field: &'static str },
    #[error(
        "additional_headers must be a JSON object in both the raw and the existing hidden params"
    )]
    HiddenHeaders,
    #[error("converting Responses JSON failed: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("converting Responses usage failed: {source}")]
    Usage {
        #[source]
        source: ResponsesUsageError,
    },
    #[error("converting a Responses tool call failed: {source}")]
    Tool {
        #[source]
        source: ToolResultsError,
    },
    #[error("converting Responses reasoning failed: {source}")]
    History {
        #[source]
        source: HistoryError,
    },
    #[error("recovering the recorded output failed: {source}")]
    Recovery {
        #[source]
        source: ResponsesSseRecoveryError,
    },
}

// ast-grep-ignore: no-one-line-functions - one conversion and error mapping for every reader
pub(crate) fn serialized<T>(value: &T) -> Result<Value, ResponseChatResultError>
where
    T: Serialize,
{
    serde_json::to_value(value).map_err(|source| ResponseChatResultError::Json { source })
}
