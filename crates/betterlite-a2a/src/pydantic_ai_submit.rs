use std::collections::BTreeMap;
use std::error::Error;
use std::future::Future;
use std::time::Duration;

use betterlite_core::agents::json::AgentJson;
use error::SubmitError;
use poll::poll_for_completion;
use serde::{Deserialize as _, Serialize};
use wire::{JsonRpcRequest, RpcParams, TaskProgress};

use crate::pydantic_ai_transformation::request::build_send_params;

pub mod error;
mod poll;
pub(crate) mod tests;
pub mod wire;

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);
pub(crate) const POLL_INTERVAL: Duration = Duration::from_millis(500);

pub struct AgentRequest<'request> {
    pub url: &'request str,
    pub headers: &'request BTreeMap<String, String>,
    pub body: &'request JsonRpcRequest,
    pub timeout: Duration,
}

pub struct AgentResponse {
    pub status: u16,
    pub body: AgentJson,
}

pub trait AgentTransport: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn post(
        &self,
        request: &AgentRequest<'_>,
    ) -> impl Future<Output = Result<AgentResponse, Self::Error>> + Send;

    fn pause(&self, duration: Duration) -> impl Future<Output = ()> + Send;
}

pub struct AgentCall<'call, P> {
    pub api_base: &'call str,
    pub request_id: &'call str,
    pub params: &'call P,
    pub timeout: Duration,
    pub headers: &'call BTreeMap<String, String>,
}

struct Dispatch<'call> {
    endpoint: &'call str,
    request_id: &'call str,
    headers: BTreeMap<String, String>,
    timeout: Duration,
}

async fn exchange<T>(
    transport: &T,
    dispatch: &Dispatch<'_>,
    body: &JsonRpcRequest,
) -> Result<(AgentJson, TaskProgress), SubmitError<T::Error>>
where
    T: AgentTransport,
{
    let response = transport
        .post(&AgentRequest {
            url: dispatch.endpoint,
            headers: &dispatch.headers,
            body,
            timeout: dispatch.timeout,
        })
        .await
        .map_err(SubmitError::Transport)?;
    if !(200..300).contains(&response.status) {
        return Err(SubmitError::HttpStatus {
            status: response.status,
        });
    }
    if !response.body.is_object() {
        return Err(SubmitError::ResponseNotObject);
    }
    let progress = TaskProgress::deserialize(&response.body).map_err(SubmitError::Response)?;
    Ok((response.body, progress))
}

pub async fn send_and_poll_raw<T, P>(
    transport: &T,
    call: &AgentCall<'_, P>,
) -> Result<AgentJson, SubmitError<T::Error>>
where
    T: AgentTransport,
    P: Serialize + Sync,
{
    let request = JsonRpcRequest {
        jsonrpc: "2.0",
        id: call.request_id.to_owned(),
        method: "message/send",
        params: RpcParams::Send(build_send_params(call.params)?),
    };
    let mut headers = call.headers.clone();
    headers.insert("Content-Type".to_owned(), "application/json".to_owned());
    let dispatch = Dispatch {
        // FastA2A serves the root endpoint, not /messages
        endpoint: call.api_base.trim_end_matches('/'),
        request_id: call.request_id,
        headers,
        timeout: call.timeout,
    };
    let (raw, progress) = exchange(transport, &dispatch, &request).await?;
    let task_id = progress.result.id;
    if progress.result.status.state == "completed" || task_id.is_empty() {
        return Ok(raw);
    }
    poll_for_completion(transport, &dispatch, &task_id).await
}
