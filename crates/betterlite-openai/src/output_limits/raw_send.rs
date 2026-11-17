use std::time::Duration;

use async_trait::async_trait;
use http::HeaderMap;
use serde_json::{Map, Value};

use super::build::{Recovery, build_output_token_limit_response};
use super::is_output_token_limit_error;

// the openai SDK's with_raw_response.create(...).parse(): the parsed answer plus the response's own headers
#[derive(Debug, Clone)]
pub struct RawChatResponse {
    pub headers: HeaderMap,
    pub body: Value,
}

#[derive(Debug, Clone)]
pub enum RawChatError {
    // the SDK's BadRequestError: its message is matched and its response travels on for a caller's error reply
    BadRequest {
        message: String,
        headers: HeaderMap,
        body: Value,
    },
    // the SDK's APITimeoutError, which carries no response
    Timeout {
        message: String,
    },
    Other {
        message: String,
    },
}

pub trait SyncRawChatTransport {
    fn create(
        &self,
        data: &Map<String, Value>,
        timeout: f64,
    ) -> Result<RawChatResponse, RawChatError>;
}

#[async_trait]
pub trait AsyncRawChatTransport {
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
    match transport.create(data, timeout) {
        Ok(response) => Ok(response),
        Err(RawChatError::BadRequest {
            message,
            headers,
            body,
        }) if is_output_token_limit_error(&message) => recover(data, headers, recovery).await,
        Err(error) => Err(error),
    }
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
    match transport.create(data, timeout).await {
        Ok(response) => Ok(response),
        Err(RawChatError::Timeout { message }) => Err(RawChatError::Timeout {
            message: timeout_message(&message, timeout, (now)().saturating_sub(started)),
        }),
        Err(RawChatError::BadRequest {
            message,
            headers,
            body,
        }) if is_output_token_limit_error(&message) => recover(data, headers, recovery).await,
        Err(error) => Err(error),
    }
}

async fn recover(
    data: &Map<String, Value>,
    headers: HeaderMap,
    recovery: &Recovery<'_>,
) -> Result<RawChatResponse, RawChatError> {
    let response = build_output_token_limit_response(data, headers, recovery)
        .await
        .map_err(|error| RawChatError::Other {
            message: error.to_string(),
        })?;
    let body = serde_json::to_value(&response.body).map_err(|error| RawChatError::Other {
        message: error.to_string(),
    })?;
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
