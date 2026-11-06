use betterlite_core::agents::json::AgentJson;

use super::error::SubmitError;
use super::wire::{JsonRpcRequest, RpcParams, TaskState};
use super::{AgentTransport, Dispatch, POLL_INTERVAL, exchange};

pub(super) const MAX_POLL_ATTEMPTS: u32 = 30;

pub(super) async fn poll_for_completion<T>(
    transport: &T,
    dispatch: &Dispatch<'_>,
    task_id: &str,
) -> Result<AgentJson, SubmitError<T::Error>>
where
    T: AgentTransport,
{
    for attempt in 0..MAX_POLL_ATTEMPTS {
        let poll_request = JsonRpcRequest {
            jsonrpc: "2.0",
            id: format!("{}-poll-{attempt}", dispatch.request_id),
            method: "tasks/get",
            params: RpcParams::Get {
                id: task_id.to_owned(),
            },
        };
        let (raw, progress) = exchange(transport, dispatch, &poll_request).await?;
        match progress.result.status.state {
            TaskState::Completed => return Ok(raw),
            state @ (TaskState::Failed | TaskState::Canceled) => {
                return Err(SubmitError::TaskEnded {
                    task_id: task_id.to_owned(),
                    state,
                });
            }
            TaskState::Other => transport.pause(POLL_INTERVAL).await,
        }
    }
    Err(SubmitError::PollTimeout {
        task_id: task_id.to_owned(),
        waited: POLL_INTERVAL.saturating_mul(MAX_POLL_ATTEMPTS),
    })
}
