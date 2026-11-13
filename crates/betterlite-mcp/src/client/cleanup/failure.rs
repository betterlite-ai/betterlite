use std::collections::VecDeque;
use std::sync::Arc;

use thiserror::Error;

// REQUEST_TIMEOUT in the MCP SDK's JSON-RPC error codes
pub const SDK_READ_TIMEOUT_CODE: i32 = -32001;

#[derive(Debug)]
pub enum Failure<Source> {
    Cancelled,
    Error(Arc<Source>),
    Group(Vec<Self>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorContext {
    Timeout,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpError {
    pub code: i32,
    pub message: String,
    pub context: Option<ErrorContext>,
}

#[derive(Debug, Error, PartialEq, Eq)]
#[error("{message}")]
pub struct ReadTimeout {
    pub message: String,
}

pub fn first_non_cancelled_cause<Source>(
    failure: &Failure<Source>,
    cleanup_errors: &[Arc<Source>],
) -> Option<Arc<Source>> {
    let mut queue = VecDeque::from([failure]);
    while let Some(current) = queue.pop_front() {
        match current {
            Failure::Group(nested) => queue.extend(nested),
            Failure::Error(error)
                if !cleanup_errors
                    .iter()
                    .any(|recorded| Arc::ptr_eq(recorded, error)) =>
            {
                return Some(Arc::clone(error));
            }
            Failure::Error(_) | Failure::Cancelled => {}
        }
    }
    None
}

// The SDK raises its own timeout while handling a TimeoutError, so a relayed upstream error with the same code has no timeout context
#[must_use]
pub fn as_mcp_read_timeout(error: &McpError) -> Option<ReadTimeout> {
    if error.code != SDK_READ_TIMEOUT_CODE || error.context != Some(ErrorContext::Timeout) {
        return None;
    }
    Some(ReadTimeout {
        message: error.message.clone(),
    })
}
