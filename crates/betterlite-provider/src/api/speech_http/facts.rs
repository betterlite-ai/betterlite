use std::str::from_utf8;
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;

use super::Mode;
use super::error::{FailureFacts, ResponseFacts};
use crate::api::speech_provider_contract::SpeechHeaders;

// The message advertises text_body and binary_body even though neither is a body branch
pub(super) const MISSING_BODY: &str = "No body found in request_data. Must provide one of: dict_body, ssml_body, text_body, binary_body";

// What the timeout message needs from the send that just failed
#[derive(Debug, Clone, Copy)]
pub(super) struct Sent {
    pub(super) mode: Mode,
    pub(super) passed: Option<Duration>,
    pub(super) effective: Duration,
    pub(super) elapsed: Duration,
}

// An httpx status error carries the response: its text and its headers less content-encoding, with content-length recomputed
pub(super) fn status(reply: &UpstreamResponse) -> FailureFacts {
    let text = body_text(&reply.body);
    let mut headers = error_headers(&reply.headers);
    if !reply.body.is_empty() {
        headers.insert("content-length".to_owned(), reply.body.len().to_string());
    }
    FailureFacts {
        response_status: Some(i64::from(reply.status)),
        status: None,
        text: None,
        headers: None,
        response: Some(ResponseFacts {
            text: Some(text),
            headers: Some(headers),
        }),
        display: String::new(),
    }
}

// Both httpx clients turn a timeout into litellm.Timeout; any other transport failure carries no status and no response
pub(super) fn transport(error: &TransportError, sent: &Sent) -> FailureFacts {
    if matches!(
        error,
        TransportError::ConnectTimeout(_) | TransportError::Deadline(_)
    ) {
        return FailureFacts {
            response_status: None,
            status: Some(408),
            text: Some(timeout_message(sent)),
            headers: Some(SpeechHeaders::new()),
            response: None,
            display: String::new(),
        };
    }
    FailureFacts {
        response_status: None,
        status: None,
        text: None,
        headers: None,
        response: None,
        display: error.to_string(),
    }
}

// The ValueError raised inside the send's try has no status, no headers and its own message
pub(super) fn missing_body() -> FailureFacts {
    FailureFacts {
        response_status: None,
        status: None,
        text: Some(MISSING_BODY.to_owned()),
        headers: None,
        response: None,
        display: String::new(),
    }
}

// The sync message prints the argument the caller passed, the async one the timeout the client applied and the time it took
fn timeout_message(sent: &Sent) -> String {
    match sent.mode {
        Mode::Sync => format!(
            "litellm.Timeout: Connection timed out after {} seconds.",
            format_timeout(sent.passed)
        ),
        Mode::Asynchronous => format!(
            "litellm.Timeout: Connection timed out. Timeout passed={}, time taken={}.{:03} seconds",
            format_timeout(Some(sent.effective)),
            sent.elapsed.as_secs(),
            sent.elapsed.subsec_millis(),
        ),
    }
}

// A whole number of seconds renders as the Python float the caller annotated, a fraction through its shortest form
pub(super) fn format_timeout(timeout: Option<Duration>) -> String {
    match timeout {
        None => "None".to_owned(),
        Some(whole) if whole.subsec_nanos() == 0 => format!("{}.0", whole.as_secs()),
        Some(fraction) => fraction.as_secs_f64().to_string(),
    }
}

fn body_text(body: &[u8]) -> String {
    from_utf8(body).map_or_else(
        |_| format!("{} bytes that are not UTF-8", body.len()),
        ToOwned::to_owned,
    )
}

fn error_headers(headers: &http::HeaderMap) -> SpeechHeaders {
    headers
        .iter()
        .filter(|(name, _)| !matches!(name.as_str(), "content-encoding" | "content-length"))
        .map(|(name, value)| (name.as_str().to_owned(), header_text(value)))
        .collect()
}

fn header_text(value: &http::HeaderValue) -> String {
    value.to_str().map_or_else(
        |_| format!("{} bytes that are not UTF-8", value.as_bytes().len()),
        ToOwned::to_owned,
    )
}
