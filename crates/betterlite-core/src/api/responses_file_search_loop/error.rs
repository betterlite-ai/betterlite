use std::convert::Infallible;

use thiserror::Error;

use crate::api::api_boundaries::ApiLimitError;
use crate::api::responses_file_search_tools::error::FileSearchToolsError;

// one error type for every stage of the emulated loop, widened as the port's error joins it
#[derive(Debug, Error)]
pub enum FileSearchLoopError<FollowUpError = Infallible> {
    #[error(transparent)]
    Tools(#[from] FileSearchToolsError),
    #[error(transparent)]
    Limit(#[from] ApiLimitError),
    #[error("the provider call that starts the file search emulation failed")]
    FirstCall(#[source] FollowUpError),
    #[error("the follow-up provider call failed")]
    FollowUp(#[source] FollowUpError),
    #[error(
        "the file search tool call arguments are neither a JSON object nor a JSON object string"
    )]
    ArgumentsNotObject,
    #[error("the provider response could not be read as an output item")]
    Wire {
        #[source]
        source: serde_json::Error,
    },
}

// widens a stage error to the port's error type; a From impl would collide with core's From<T> for T
#[must_use]
pub fn widen<FollowUpError>(
    error: FileSearchLoopError<Infallible>,
) -> FileSearchLoopError<FollowUpError> {
    match error {
        FileSearchLoopError::Tools(source) => FileSearchLoopError::Tools(source),
        FileSearchLoopError::Limit(source) => FileSearchLoopError::Limit(source),
        FileSearchLoopError::FirstCall(never) | FileSearchLoopError::FollowUp(never) => {
            match never {}
        }
        FileSearchLoopError::ArgumentsNotObject => FileSearchLoopError::ArgumentsNotObject,
        FileSearchLoopError::Wire { source } => FileSearchLoopError::Wire { source },
    }
}
