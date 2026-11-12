use std::str::Utf8Error;

use thiserror::Error;

use crate::api::api_boundaries::ApiLimitError;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CustomToolsError {
    #[error("allowed_callers must be a list of strings")]
    AllowedCallers,
    #[error("the tool call arguments are not UTF-8")]
    ArgumentsNotUtf8 {
        #[source]
        source: Utf8Error,
    },
    #[error("the tool call arguments are over the byte limit")]
    ArgumentsTooLarge {
        #[source]
        source: ApiLimitError,
    },
    #[error("item {position} of the record is not a [name, value] pair")]
    NotAPair { position: usize },
    #[error("the input item role must be a string")]
    RoleNotText,
}
