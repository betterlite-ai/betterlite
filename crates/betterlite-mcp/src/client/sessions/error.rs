use std::error::Error;
use std::sync::Arc;

use thiserror::Error;
use tokio::task::JoinError;

use crate::client::cleanup::failure::{ErrorContext, Failure, ReadTimeout};

use super::stream::StreamFailure;

#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error("{message}")]
pub struct McpFailure {
    pub code: i32,
    pub message: String,
    pub context: Option<ErrorContext>,
}

#[derive(Debug, Error)]
pub enum SessionConfigError {
    #[error("stdio_config is required for stdio transport")]
    MissingStdioConfig,
    #[error("protocol_version {value} is not a supported MCP protocol version")]
    ProtocolVersion { value: String },
}

#[derive(Debug, Error)]
pub enum ExitFailure<Source>
where
    Source: Error + Send + Sync + 'static,
{
    #[error("the MCP cleanup was cancelled")]
    Cancelled,
    #[error("the MCP cleanup failed")]
    Failure(#[source] Arc<Source>),
    #[error("the MCP cleanup failed")]
    Group(Vec<Self>),
}

impl<Source: Error + Send + Sync + 'static> From<Source> for ExitFailure<Source> {
    fn from(source: Source) -> Self {
        Self::Failure(Arc::new(source))
    }
}

#[derive(Debug, Error)]
pub enum SessionError<Source>
where
    Source: Error + Send + Sync + 'static,
{
    #[error("the MCP server did not accept the session")]
    Protocol(#[source] McpFailure),
    #[error("the MCP runtime failed")]
    Runtime(#[source] Arc<Source>),
    #[error("the MCP response stream failed")]
    Stream(#[source] Arc<StreamFailure<Source>>),
    #[error("the caller cancelled the MCP session")]
    Cancelled,
    #[error("the MCP session cleanup timed out")]
    CleanupTimedOut,
    #[error("the MCP cleanup task failed")]
    CleanupTask(#[source] JoinError),
    #[error(transparent)]
    ReadTimeout(#[from] ReadTimeout),
    #[error(transparent)]
    Configuration(#[from] SessionConfigError),
}

#[must_use]
pub fn to_failure<Source>(failure: ExitFailure<Source>) -> Failure<Source>
where
    Source: Error + Send + Sync + 'static,
{
    match failure {
        ExitFailure::Cancelled => Failure::Cancelled,
        ExitFailure::Failure(source) => Failure::Error(source),
        ExitFailure::Group(nested) => Failure::Group(nested.into_iter().map(to_failure).collect()),
    }
}
