use std::error::Error;
use std::time::Duration;

use thiserror::Error;

use super::wire::TaskState;
use crate::pydantic_ai_transformation::request::ParamsError;

#[derive(Debug, Error)]
pub enum SubmitError<E>
where
    E: Error + 'static,
{
    #[error(transparent)]
    Params(#[from] ParamsError),
    #[error("the Pydantic AI agent request failed")]
    Transport(#[source] E),
    #[error("the Pydantic AI agent answered with HTTP status {status}")]
    HttpStatus { status: u16 },
    #[error("the Pydantic AI agent response is not a JSON object")]
    ResponseNotObject,
    #[error("the Pydantic AI agent response is not a valid task response")]
    Response(#[source] serde_json::Error),
    #[error("Task {task_id} ended with state: {state}")]
    TaskEnded { task_id: String, state: TaskState },
    #[error("Task {task_id} did not complete within {:.1} seconds", .waited.as_secs_f64())]
    PollTimeout { task_id: String, waited: Duration },
}
