use std::error::Error as StdError;

use thiserror::Error;

use crate::anthropic_invokes::documents::FileMessageError;
use crate::anthropic_results::images::ImageError;
use crate::anthropic_results::tool_result::ToolResultError;

#[derive(Debug, Error)]
pub enum UserBlocksError<FileFailure>
where
    FileFailure: StdError + Send + Sync + 'static,
{
    #[error(transparent)]
    Image(#[from] ImageError),
    #[error(transparent)]
    File(#[from] FileMessageError),
    #[error(transparent)]
    ToolResult(#[from] ToolResultError<FileFailure>),
    #[error("message at index {index} is not an object")]
    MessageNotObject { index: usize },
    #[error("message at index {index} has no string role")]
    MissingRole { index: usize },
    #[error("content block {block} of message {message} is not an object")]
    BlockNotObject { message: usize, block: usize },
    #[error("content block {block} of message {message} needs a string {field}")]
    Field {
        message: usize,
        block: usize,
        field: &'static str,
    },
    #[error("message at index {index} cannot be read as an OpenAI message")]
    MessageDecode {
        index: usize,
        #[source]
        source: serde_json::Error,
    },
}
