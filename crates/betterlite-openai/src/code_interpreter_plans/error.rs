use std::error::Error as StdError;

use thiserror::Error;

use crate::sandbox_lifecycle::lifecycle::LifecycleError;
use crate::sandbox_lifecycle::tool_execution::ToolExecutionError;

#[derive(Debug, Error)]
pub enum PlanError<E>
where
    E: StdError + Send + Sync + 'static,
{
    #[error(transparent)]
    Lifecycle(#[from] LifecycleError<E>),
    #[error(transparent)]
    ToolExecution(#[from] ToolExecutionError<E>),
    #[error("the agentic loop request could not be serialized")]
    Serialize(#[from] serde_json::Error),
}
