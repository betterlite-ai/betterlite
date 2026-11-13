use thiserror::Error;

use crate::rubrik::block_decision::ResponseBlockError;
use crate::rubrik::tool_calls::ToolCallError;
use crate::rubrik_delivery::error::RubrikError;

use super::context::RequestData;

// The block LiteLLM raises as ModifyResponseException; the proxy turns it into a 200 carrying the violation text.
#[derive(Debug, Error)]
#[error("{message}")]
pub struct ModifyResponseException {
    pub message: String,
    pub model: String,
    pub request_data: Box<RequestData>,
    pub guardrail_name: String,
}

#[derive(Debug, Error)]
pub enum ModerationError {
    #[error("{0}")]
    Blocked(Box<ModifyResponseException>),
    #[error("the response moderation service returned a malformed response")]
    Malformed(#[from] ResponseBlockError),
    #[error(transparent)]
    Rubrik(#[from] RubrikError),
    #[error("a tool call could not be normalized")]
    ToolCall(#[from] ToolCallError),
    #[error("the {field} value has the wrong type")]
    FieldType { field: &'static str },
    #[error("the clock reading does not fit a Unix timestamp")]
    Timestamp,
}
