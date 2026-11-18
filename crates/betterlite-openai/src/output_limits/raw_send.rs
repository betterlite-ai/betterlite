use std::time::Duration;

use async_trait::async_trait;
use http::HeaderMap;
use serde_json::{Map, Value};
use thiserror::Error;

use super::ErrorBody;
use super::build::{Recovery, build_limit_response};
use super::tokens::PromptCountError;

// the openai SDK's with_raw_response.create(...).parse(): the parsed answer plus the response's own headers
#[derive(Debug, Clone)]
pub struct RawChatResponse {
    pub headers: HeaderMap,
    pub body: Value,
}

// the SDK's BadRequestError payload: its message is matched and its response travels on for a caller's error reply
#[derive(Debug, Clone)]
pub struct BadRequestReply {
    pub message: String,
    pub headers: HeaderMap,
    pub body: Value,
}

// the SDK's other APIStatusError payloads keep their status and response, which LiteLLM's bare `raise e` preserves
#[derive(Debug, Clone)]
pub struct StatusReply {
    pub status: u16,
    pub message: String,
    pub headers: HeaderMap,
    pub body: Value,
}

#[derive(Debug, Error)]
pub enum RawChatError {
    // boxed so the word-sized failures stay within 3x of each other, the limit variant_size_differences enforces
    #[error("{}", .0.message)]
    BadRequest(Box<BadRequestReply>),
    #[error("{}", .0.message)]
    Status(Box<StatusReply>),
    // the SDK's APITimeoutError, which carries no response
    #[error("{message}")]
    Timeout { message: String },
    #[error("{message}")]
    Other { message: String },
    #[error("the recovery prompt could not be counted: {source}")]
    Count {
        #[source]
        source: Box<PromptCountError>,
    },
    #[error("the recovered response could not be encoded: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}

pub trait SyncRawChatTransport: Send + Sync {
    fn create(
        &self,
        data: &Map<String, Value>,
        timeout: f64,
    ) -> Result<RawChatResponse, RawChatError>;
}

#[async_trait]
pub trait AsyncRawChatTransport: Send + Sync {
    async fn create(
        &self,
        data: &Map<String, Value>,
        timeout: f64,
    ) -> Result<RawChatResponse, RawChatError>;
}

// LiteLLM's make_sync_azure_openai_chat_completion_request: only the full output-limit sentence turns the 400 into a reply
pub async fn send_raw_chat_sync(
    transport: &dyn SyncRawChatTransport,
    data: &Map<String, Value>,
    timeout: f64,
    recovery: &Recovery<'_>,
) -> Result<RawChatResponse, RawChatError> {
    answer(
        transport.create(data, timeout),
        |message| RawChatError::Timeout { message },
        data,
        recovery,
    )
    .await
}

// LiteLLM's make_azure_openai_chat_completion_request: the timeout keeps its elapsed time, the output limit recovers
pub async fn send_raw_chat_async(
    transport: &dyn AsyncRawChatTransport,
    data: &Map<String, Value>,
    timeout: f64,
    now: fn() -> Duration,
    recovery: &Recovery<'_>,
) -> Result<RawChatResponse, RawChatError> {
    let started = (now)();
    let created = transport.create(data, timeout).await;
    let timed_out = |message: String| RawChatError::Timeout {
        message: timeout_message(&message, timeout, (now)().saturating_sub(started)),
    };
    answer(created, timed_out, data, recovery).await
}

// only a BadRequestError whose message carries the whole output-limit sentence becomes a reply; the rest travels on
async fn answer<F>(
    created: Result<RawChatResponse, RawChatError>,
    timed_out: F,
    data: &Map<String, Value>,
    recovery: &Recovery<'_>,
) -> Result<RawChatResponse, RawChatError>
where
    F: Fn(String) -> RawChatError,
{
    match created {
        Ok(response) => Ok(response),
        Err(RawChatError::Timeout { message }) => Err(timed_out(message)),
        Err(RawChatError::BadRequest(rejected))
            if ErrorBody::Http(rejected.message.as_bytes()).reaches_output_limit() =>
        {
            let BadRequestReply { headers, .. } = *rejected;
            recover(data, headers, recovery).await
        }
        Err(error) => Err(error),
    }
}

async fn recover(
    data: &Map<String, Value>,
    headers: HeaderMap,
    recovery: &Recovery<'_>,
) -> Result<RawChatResponse, RawChatError> {
    let response = build_limit_response(data, headers, recovery)
        .await
        .map_err(|source| RawChatError::Count {
            source: Box::new(source),
        })?;
    let body =
        serde_json::to_value(&response.body).map_err(|source| RawChatError::Encode { source })?;
    Ok(RawChatResponse {
        headers: response.headers,
        body,
    })
}

fn timeout_message(message: &str, timeout: f64, elapsed: Duration) -> String {
    format!(
        "{message} - timeout value={timeout}, time taken={} seconds",
        seconds_text(elapsed)
    )
}

// Python prints the rounded float; the local decimal keeps the same two places without float arithmetic
fn seconds_text(elapsed: Duration) -> String {
    let hundredths = elapsed.as_millis().saturating_add(5).saturating_div(10);
    format!(
        "{}.{:02}",
        hundredths.saturating_div(100),
        hundredths.rem_euclid(100)
    )
}
