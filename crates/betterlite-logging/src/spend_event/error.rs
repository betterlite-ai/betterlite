use thiserror::Error;

use crate::spend_event::usage::UsageError;

#[derive(Debug, Error)]
pub enum SpendEventBuildError {
    #[error("the callback kwargs fail the projection: {source}")]
    Kwargs {
        #[source]
        source: serde_json::Error,
    },
    #[error("the spend event cannot be encoded: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the combined usage is invalid: {source}")]
    Usage {
        #[source]
        source: UsageError,
    },
    #[error("the response cost {value} is not a decimal number: {source}")]
    ResponseCost {
        value: String,
        #[source]
        source: rust_decimal::Error,
    },
}

#[derive(Debug, Error)]
pub enum SpendEventDecodeError {
    #[error("the spend event line is {length} bytes, over the {limit} byte limit")]
    TooLarge { length: usize, limit: usize },
    #[error("the line is not a version 1 spend event: {source}")]
    Event {
        #[source]
        source: serde_json::Error,
    },
}
