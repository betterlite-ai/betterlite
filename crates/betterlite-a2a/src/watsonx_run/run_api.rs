use std::error::Error;

use serde::Serialize;
use thiserror::Error;

use super::event::RunEvent;
use super::transport::{RunRequest, RunTransport};

mod tests;

#[derive(Clone, Debug, Serialize)]
pub struct RunBody {
    pub agent_id: String,
    pub message: RunMessage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct RunMessage {
    pub role: &'static str,
    pub content: Vec<RunContent>,
}

#[derive(Clone, Debug, Serialize)]
pub struct RunContent {
    pub response_type: &'static str,
    pub text: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Method {
    Post,
    Get,
}

#[derive(Debug, Error)]
pub enum ApiError<E>
where
    E: Error + 'static,
{
    #[error("the watsonx Orchestrate run request failed")]
    Transport(#[source] E),
    #[error("the watsonx Orchestrate runs API answered with HTTP status {status}")]
    Status { status: u16 },
    #[error("the watsonx Orchestrate run answer is not the expected JSON")]
    Decode(#[source] serde_json::Error),
    #[error("the watsonx Orchestrate run answer passed {max} bytes")]
    TooLarge { max: usize },
}

pub async fn call_run<T>(
    transport: &T,
    request: &RunRequest<'_>,
    method: Method,
) -> Result<RunEvent, ApiError<T::Error>>
where
    T: RunTransport,
{
    let response = match method {
        Method::Post => transport.post(request).await,
        Method::Get => transport.get(request).await,
    }
    .map_err(ApiError::Transport)?;
    if !(200..300).contains(&response.status) {
        return Err(ApiError::Status {
            status: response.status,
        });
    }
    if response.body.len() > request.limits.max_response_bytes {
        return Err(ApiError::TooLarge {
            max: request.limits.max_response_bytes,
        });
    }
    serde_json::from_slice(&response.body).map_err(ApiError::Decode)
}
