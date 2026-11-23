use std::error::Error;

use betterlite_a2a::client::error::ClientError;
use betterlite_a2a::client::retry::SendError;
use betterlite_a2a::errors::{A2AError, LLM_PROVIDER};
use betterlite_a2a::send::error::{SendCallError, SendEntryError};
use pyo3::PyErr;
use pyo3::exceptions::PyRuntimeError;
use serde::Serialize;

use super::error::NativeA2aError;

const BASE_STATUS: u16 = 500;
const CONNECTION_STATUS: u16 = 503;
const SEND_NO_RESPONSE: &str =
    "A2A send_message failed: no response received after retry attempts.";
const STREAM_NO_RESPONSE: &str =
    "A2A send_message_streaming failed: no response received after retry attempts.";

// Every failure the bridge hands Python, in the fields LiteLLM's A2A exceptions keep.
#[derive(Debug, Serialize)]
pub(crate) struct ErrorPayload {
    #[serde(rename = "type")]
    pub(crate) kind: &'static str,
    pub(crate) message: String,
    pub(crate) status_code: Option<u16>,
    pub(crate) llm_provider: Option<&'static str>,
    pub(crate) url: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) litellm_debug_info: Option<String>,
    pub(crate) max_retries: Option<u32>,
    pub(crate) num_retries: Option<u32>,
    pub(crate) base_url: Option<String>,
    pub(crate) localhost_url: Option<String>,
    pub(crate) failures: Option<Vec<FailurePayload>>,
    pub(crate) original_error: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct FailurePayload {
    pub(crate) path: String,
    pub(crate) error: String,
}

impl ErrorPayload {
    pub(crate) fn empty(kind: &'static str, message: String) -> Self {
        Self {
            kind,
            message,
            status_code: None,
            llm_provider: None,
            url: None,
            model: None,
            litellm_debug_info: None,
            max_retries: None,
            num_retries: None,
            base_url: None,
            localhost_url: None,
            failures: None,
            original_error: None,
        }
    }

    pub(crate) fn base(message: String) -> Self {
        Self {
            status_code: Some(BASE_STATUS),
            llm_provider: Some(LLM_PROVIDER),
            ..Self::empty("A2AError", message)
        }
    }

    fn connection(message: String, url: Option<String>) -> Self {
        Self {
            status_code: Some(CONNECTION_STATUS),
            llm_provider: Some(LLM_PROVIDER),
            url,
            ..Self::empty("A2AConnectionError", message)
        }
    }

    pub(crate) fn from_a2a(error: &A2AError) -> Self {
        let status_code = Some(error.status_code());
        let url = error.url().map(str::to_owned);
        match error {
            A2AError::Base {
                message,
                model,
                debug_info,
                retries,
            } => Self {
                status_code,
                llm_provider: Some(LLM_PROVIDER),
                url,
                model: model.clone(),
                litellm_debug_info: debug_info.clone(),
                max_retries: retries.max,
                num_retries: retries.retried,
                ..Self::empty("A2AError", message.clone())
            },
            A2AError::Connection {
                message,
                model,
                debug_info,
                retries,
                ..
            } => Self {
                status_code,
                llm_provider: Some(LLM_PROVIDER),
                url,
                model: model.clone(),
                litellm_debug_info: debug_info.clone(),
                max_retries: retries.max,
                num_retries: retries.retried,
                ..Self::empty("A2AConnectionError", message.clone())
            },
            A2AError::AgentCard {
                message,
                status_code: card_status,
                ..
            } => Self {
                status_code: Some(*card_status),
                llm_provider: Some(LLM_PROVIDER),
                url,
                ..Self::empty("A2AAgentCardError", message.clone())
            },
            A2AError::AgentCardDiscovery {
                base_url, attempts, ..
            } => Self {
                status_code,
                llm_provider: Some(LLM_PROVIDER),
                url: Some(base_url.clone()),
                base_url: Some(base_url.clone()),
                failures: Some(
                    attempts
                        .failures
                        .iter()
                        .map(|failure| FailurePayload {
                            path: failure.path.clone(),
                            error: failure.reason.to_string(),
                        })
                        .collect(),
                ),
                ..Self::empty(
                    "A2AAgentCardDiscoveryError",
                    format!(
                        "Failed to fetch agent card from {base_url}. Tried {}",
                        attempts.render()
                    ),
                )
            },
            A2AError::LocalhostUrl {
                localhost_url,
                base_url,
                model,
                original_error,
            } => Self {
                status_code,
                llm_provider: Some(LLM_PROVIDER),
                url,
                model: model.clone(),
                base_url: Some(base_url.clone()),
                localhost_url: Some(localhost_url.clone()),
                original_error: original_error.as_ref().map(|source| source.to_string()),
                ..Self::empty(
                    "A2ALocalhostURLError",
                    format!(
                        "Agent card contains localhost/internal URL '{localhost_url}'. Retrying with base URL '{base_url}'."
                    ),
                )
            },
        }
    }

    // A panic here would surface as a Python exception with no error JSON, so encoding never fails the call.
    pub(crate) fn into_exception(self) -> PyErr {
        match serde_json::to_string(&self) {
            Ok(text) => NativeA2aError::new_err(text),
            Err(_encoding) => PyRuntimeError::new_err(self.message),
        }
    }
}

pub(crate) fn send_payload(error: SendCallError, streaming: bool) -> ErrorPayload {
    let message = error.to_string();
    match error {
        SendCallError::MissingClient => ErrorPayload::empty("ValueError", message),
        SendCallError::Client(source) => ErrorPayload::from_a2a(&source),
        SendCallError::Send(source) => send_failure(source, streaming),
        SendCallError::NotText { .. }
        | SendCallError::Json { .. }
        | SendCallError::Usage(_)
        | SendCallError::TokenCount(_)
        | SendCallError::Logging(_)
        | SendCallError::Response(_) => ErrorPayload::base(message),
    }
}

fn send_failure(error: SendError, streaming: bool) -> ErrorPayload {
    match error {
        SendError::A2A(source) => ErrorPayload::from_a2a(&source),
        SendError::Client(source) => {
            ErrorPayload::connection(source.to_string(), client_url(&source))
        }
        SendError::Recovery(source) => ErrorPayload::empty("RuntimeError", source.to_string()),
        SendError::NoResponse => ErrorPayload::empty(
            "RuntimeError",
            String::from(if streaming {
                STREAM_NO_RESPONSE
            } else {
                SEND_NO_RESPONSE
            }),
        ),
    }
}

// LiteLLM's connection error names the URL the agent refused, and the transport error carries the one it used.
fn client_url(error: &ClientError) -> Option<String> {
    match error {
        ClientError::Connection { url, .. }
        | ClientError::Timeout { url, .. }
        | ClientError::Request { url, .. }
        | ClientError::Status { url, .. }
        | ClientError::JsonRpc { url, .. }
        | ClientError::Decode { url, .. }
        | ClientError::Encode { url, .. }
        | ClientError::Protocol { url }
        | ClientError::EventTooLarge { url }
        | ClientError::Endpoint { url, .. } => Some(url.clone()),
        ClientError::SseError { .. }
        | ClientError::Final { .. }
        | ClientError::InvalidHeaderName { .. }
        | ClientError::InvalidHeaderValue { .. }
        | ClientError::Convert(_)
        | ClientError::MissingEndpoint
        | ClientError::NoTransports => None,
    }
}

pub(crate) fn entry_error<C, P>(error: SendEntryError<C, P>, streaming: bool) -> PyErr
where
    C: Error + 'static,
    P: Error + 'static,
{
    match error {
        SendEntryError::Call(source) => send_payload(source, streaming).into_exception(),
        SendEntryError::Bridge(source) => {
            ErrorPayload::empty("RuntimeError", source.to_string()).into_exception()
        }
    }
}
