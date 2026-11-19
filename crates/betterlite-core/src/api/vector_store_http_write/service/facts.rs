use std::str::from_utf8;
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use serde_json::Number;

use super::super::error::{FailureFacts, HeaderMap, ResponseFacts, header_text};
use super::{ClientKind, Operation};

// What the timeout message needs from the send that just failed
#[derive(Debug, Clone, Copy)]
pub(super) struct Sent {
    pub(super) mode: ClientKind,
    pub(super) operation: Operation,
    pub(super) timeout: Duration,
    pub(super) elapsed: Duration,
}

pub(super) fn http_response(reply: &UpstreamResponse) -> http::Response<Vec<u8>> {
    let mut response = http::Response::new(reply.body.clone());
    *response.status_mut() =
        http::StatusCode::from_u16(reply.status).unwrap_or(http::StatusCode::OK);
    *response.headers_mut() = reply.headers.clone();
    response
}

// An httpx.HTTPStatusError carries the masked response: its text and its headers less the two content headers
pub(super) fn status_facts(reply: &UpstreamResponse) -> FailureFacts {
    let text = match from_utf8(&reply.body) {
        Ok(text) => text.to_owned(),
        Err(_) => format!("{} bytes that are not UTF-8", reply.body.len()),
    };
    let headers = error_headers(&reply.headers);
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

pub(super) fn transport_facts(error: &TransportError, sent: &Sent) -> FailureFacts {
    if caught_timeout(error, sent.mode, sent.operation) {
        return FailureFacts {
            response_status: None,
            status: Some(Json::Number(Number::from(408_u16))),
            text: Some(timeout_message(sent)),
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

// Only the update handlers and the sync delete handler catch httpx.TimeoutException; the async delete does not
fn caught_timeout(error: &TransportError, mode: ClientKind, operation: Operation) -> bool {
    let timed_out = matches!(
        error,
        TransportError::ConnectTimeout(_) | TransportError::Deadline(_)
    );
    timed_out && !(mode == ClientKind::Asynchronous && operation == Operation::Delete)
}

fn timeout_message(sent: &Sent) -> String {
    if sent.mode == ClientKind::Sync {
        return "litellm.Timeout: Connection timed out after None seconds.".to_owned();
    }
    format!(
        "litellm.Timeout: Connection timed out. Timeout passed={}, time taken={}.{:03} seconds",
        format_timeout(sent.timeout),
        sent.elapsed.as_secs(),
        sent.elapsed.subsec_millis(),
    )
}

fn format_timeout(timeout: Duration) -> String {
    if timeout.subsec_nanos() == 0 {
        timeout.as_secs().to_string()
    } else {
        timeout.as_secs_f64().to_string()
    }
}
