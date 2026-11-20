use std::str::from_utf8;
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use betterlite_openai::error::ProviderError;
use serde_json::Number;

use crate::api::vector_store_http_write::error::{
    FailureFacts, ResponseFacts, handle_error, header_map,
};

use super::config::VideoRemixListConfig;
use super::error::{ErrorHeaders, Operation, VideoRemixListTransportError};
use super::wire::WireError;

// The handler mode the send ran in, the timeout it was given and how long the send took
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SendMode {
    Sync,
    Asynchronous,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct SendFacts {
    pub(super) operation: Operation,
    pub(super) mode: SendMode,
    pub(super) timeout: Option<Duration>,
    pub(super) elapsed: Duration,
}

// The shared conversion: B2402's handle_error picks the status, text and headers, the provider config names the error class
fn provider_error(config: &dyn VideoRemixListConfig, facts: &FailureFacts) -> ProviderError {
    let converted = handle_error(facts, None);
    let mut error =
        config.get_error_class(&converted.message, converted.status_code, converted.headers);
    error.status_code_is_synthesized = converted.status_code_is_synthesized;
    error
}

// The body text httpx would read off the masked response, with a binary body named by its byte count
fn body_text(body: &[u8]) -> String {
    from_utf8(body).map_or_else(
        |_| format!("{} bytes that are not UTF-8", body.len()),
        ToOwned::to_owned,
    )
}

// An httpx.HTTPStatusError carries the masked response: its text and its headers less content-encoding, with content-length recomputed
fn status_facts(reply: &UpstreamResponse) -> FailureFacts {
    let text = body_text(&reply.body);
    let mut headers = header_map(&reply.headers);
    headers.remove("content-encoding");
    headers.remove("content-length");
    // httpx re-encodes the decoded body into the masked response, so a non-empty one carries a new content-length
    if !reply.body.is_empty() {
        let length = reply.body.len().to_string();
        headers.insert(String::from("content-length"), Json::Text(length));
    }
    FailureFacts {
        response_status: Some(i64::from(reply.status)),
        status: Some(Json::Number(Number::from(reply.status))),
        text: Some(text.clone()),
        headers: None,
        response: Some(ResponseFacts {
            status_code: i64::from(reply.status),
            text: Some(text),
            headers: Some(headers),
        }),
        display: String::new(),
    }
}

// A failure with no response behind it: the text is the failure's own, and a missing status becomes a synthesized 500
fn display_facts(display: String) -> FailureFacts {
    FailureFacts {
        response_status: None,
        status: None,
        text: None,
        headers: None,
        response: None,
        display,
    }
}

// Only the remix handlers catch httpx.TimeoutException and raise litellm.Timeout; a list GET lets it escape as a raw exception
fn transport_facts(facts: &SendFacts, error: &TransportError) -> FailureFacts {
    let timed_out = matches!(
        error,
        TransportError::ConnectTimeout(_) | TransportError::Deadline(_)
    );
    if timed_out && facts.operation == Operation::Remix {
        return FailureFacts {
            response_status: None,
            status: Some(Json::Number(Number::from(408_u16))),
            text: Some(timeout_message(facts)),
            headers: Some(ErrorHeaders::new()),
            response: None,
            display: String::new(),
        };
    }
    display_facts(error.to_string())
}

// The sync and async HTTP handlers word the same timeout differently, and the sync one shows the argument as it was given
pub(super) fn timeout_message(facts: &SendFacts) -> String {
    match facts.mode {
        SendMode::Sync => format!(
            "litellm.Timeout: Connection timed out after {} seconds.",
            timeout_text(facts.timeout)
        ),
        SendMode::Asynchronous => format!(
            "litellm.Timeout: Connection timed out. Timeout passed={}, time taken={}.{:03} seconds",
            timeout_text(facts.timeout),
            facts.elapsed.as_secs(),
            facts.elapsed.subsec_millis(),
        ),
    }
}

fn timeout_text(timeout: Option<Duration>) -> String {
    let Some(given) = timeout else {
        return String::from("None");
    };
    if given.subsec_nanos() == 0 {
        return given.as_secs().to_string();
    }
    given.as_secs_f64().to_string()
}

pub(super) fn status_error(
    config: &dyn VideoRemixListConfig,
    reply: &UpstreamResponse,
) -> VideoRemixListTransportError {
    let provider = provider_error(config, &status_facts(reply));
    VideoRemixListTransportError::Provider(Box::new(provider))
}

pub(super) fn wire_error(
    config: &dyn VideoRemixListConfig,
    error: &WireError,
) -> VideoRemixListTransportError {
    let provider = provider_error(config, &display_facts(error.to_string()));
    VideoRemixListTransportError::Provider(Box::new(provider))
}

pub(super) fn transport_error(
    config: &dyn VideoRemixListConfig,
    facts: &SendFacts,
    error: &TransportError,
) -> VideoRemixListTransportError {
    let provider = provider_error(config, &transport_facts(facts, error));
    VideoRemixListTransportError::Provider(Box::new(provider))
}

// Both handlers call raise_for_status on their send, so a 4xx or 5xx answer reaches error conversion before the response transform
pub(super) fn check_status(
    config: &dyn VideoRemixListConfig,
    reply: UpstreamResponse,
) -> Result<UpstreamResponse, VideoRemixListTransportError> {
    if reply.status < 400 {
        return Ok(reply);
    }
    Err(status_error(config, &reply))
}
