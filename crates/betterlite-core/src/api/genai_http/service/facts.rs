use std::str::from_utf8;
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use serde_json::Number;

use crate::api::vector_store_http_write::error::{
    FailureFacts, HeaderMap, ResponseFacts, header_text,
};
use crate::api::vector_store_http_write::service::ClientKind;

// What the timeout message needs from the send that just failed: the timeout the caller passed, the one httpx used,
// and how long the send took
pub(super) struct Attempt {
    pub(super) mode: ClientKind,
    pub(super) timeout: Option<Duration>,
    pub(super) effective: Duration,
    pub(super) elapsed: Duration,
}

// An httpx.HTTPStatusError carries the masked response: its text and its headers less content-encoding, with
// content-length recomputed
pub(super) fn status_facts(reply: &UpstreamResponse) -> FailureFacts {
    let text = from_utf8(&reply.body).map_or_else(
        |_| format!("{} bytes that are not UTF-8", reply.body.len()),
        ToOwned::to_owned,
    );
    let mut headers = error_headers(&reply.headers);
    // httpx re-encodes the decoded body into the masked response, so a non-empty one carries a new content-length
    if !reply.body.is_empty() {
        let length = reply.body.len().to_string();
        headers.insert("content-length".to_owned(), Json::Text(length));
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

fn error_headers(headers: &http::HeaderMap) -> HeaderMap {
    headers
        .iter()
        .filter(|(name, _)| !matches!(name.as_str(), "content-encoding" | "content-length"))
        .map(|(name, value)| (name.as_str().to_owned(), Json::Text(header_text(value))))
        .collect()
}

// A timeout carries 408 and the timeout message; anything else the transport raises carries no status and is
// synthesized into a 500
pub(super) fn transport_facts(error: &TransportError, attempt: &Attempt) -> FailureFacts {
    if timed_out(error) {
        return FailureFacts {
            response_status: None,
            status: Some(Json::Number(Number::from(408_u16))),
            text: Some(timeout_message(attempt)),
            headers: Some(HeaderMap::new()),
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

fn timed_out(error: &TransportError) -> bool {
    matches!(
        error,
        TransportError::ConnectTimeout(_) | TransportError::Deadline(_)
    )
}

// HTTPHandler.post catches httpx.TimeoutException in both forms: the sync one echoes the caller's timeout, the async
// one echoes the client's after substituting its own
fn timeout_message(attempt: &Attempt) -> String {
    if attempt.mode == ClientKind::Sync {
        return match attempt.timeout {
            Some(timeout) => format!(
                "litellm.Timeout: Connection timed out after {} seconds.",
                format_timeout(timeout)
            ),
            None => "litellm.Timeout: Connection timed out after None seconds.".to_owned(),
        };
    }
    format!(
        "litellm.Timeout: Connection timed out. Timeout passed={}, time taken={}.{:03} seconds",
        format_timeout(attempt.timeout.unwrap_or(attempt.effective)),
        attempt.elapsed.as_secs(),
        attempt.elapsed.subsec_millis(),
    )
}

fn format_timeout(timeout: Duration) -> String {
    if timeout.subsec_nanos() == 0 {
        timeout.as_secs().to_string()
    } else {
        timeout.as_secs_f64().to_string()
    }
}
