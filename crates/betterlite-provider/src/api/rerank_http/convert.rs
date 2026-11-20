use std::str::from_utf8;
use std::time::Duration;

use betterlite_core::api::vector_store_http_write::error::{
    FailureFacts, HeaderMap, ResponseFacts, header_text,
};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::decisions::Json;
use serde_json::Number;

use super::client::{ClientKind, RerankSendError};
use crate::api::rerank_contracts::{Headers, RerankProviderResponse};

// What the timeout message needs from the send that just failed
#[derive(Debug, Clone, Copy)]
pub(super) struct Sent {
    pub(super) kind: ClientKind,
    pub(super) timeout: Option<Duration>,
    pub(super) elapsed: Duration,
}

// An httpx.HTTPStatusError carries the masked response: its text and its headers less content-encoding, with content-length recomputed
#[must_use]
pub(super) fn status_facts(reply: &UpstreamResponse) -> FailureFacts {
    let text = from_utf8(&reply.body).map_or_else(
        |_| format!("{} bytes that are not UTF-8", reply.body.len()),
        ToOwned::to_owned,
    );
    let mut headers = masked_headers(&reply.headers);
    // httpx re-encodes the decoded body into the masked response, so a non-empty one carries a new content-length
    if !reply.body.is_empty() {
        headers.insert(
            "content-length".to_owned(),
            Json::Text(reply.body.len().to_string()),
        );
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

fn masked_headers(headers: &http::HeaderMap) -> HeaderMap {
    headers
        .iter()
        .filter(|(name, _)| !matches!(name.as_str(), "content-encoding" | "content-length"))
        .map(|(name, value)| (name.as_str().to_owned(), Json::Text(header_text(value))))
        .collect()
}

#[must_use]
pub(super) fn send_facts(error: &RerankSendError, sent: &Sent) -> FailureFacts {
    match error {
        RerankSendError::Timeout => timeout_facts(sent),
        RerankSendError::Failed { message } => FailureFacts {
            response_status: None,
            status: None,
            text: None,
            headers: None,
            response: None,
            display: message.clone(),
        },
    }
}

// The sync handler prints the timeout it was given and the async one the time the request took
fn timeout_facts(sent: &Sent) -> FailureFacts {
    let text = match sent.kind {
        ClientKind::Sync => format!(
            "litellm.Timeout: Connection timed out after {} seconds.",
            requested_timeout(sent.timeout)
        ),
        ClientKind::Asynchronous => format!(
            "litellm.Timeout: Connection timed out. Timeout passed={}, time taken={} seconds",
            requested_timeout(sent.timeout),
            elapsed_text(sent.elapsed)
        ),
    };
    FailureFacts {
        response_status: None,
        status: Some(Json::Number(Number::from(408_u16))),
        text: Some(text),
        headers: Some(HeaderMap::new()),
        response: None,
        display: String::new(),
    }
}

pub(super) fn requested_timeout(timeout: Option<Duration>) -> String {
    let Some(timeout) = timeout else {
        return "None".to_owned();
    };
    if timeout.subsec_nanos() == 0 {
        return timeout.as_secs().to_string();
    }
    timeout.as_secs_f64().to_string()
}

// Python's round(seconds, 3) renders as 0.1, 1.0 or 12.345, never with trailing zeros past the digit
pub(super) fn elapsed_text(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    let millis = elapsed.subsec_millis();
    if millis == 0 {
        return format!("{seconds}.0");
    }
    let fraction = format!("{millis:03}");
    format!("{seconds}.{}", fraction.trim_end_matches('0'))
}

// The raw_response the provider transformer reads: the whole answer, status and body as the provider sent them
#[must_use]
pub(super) fn provider_response(reply: &UpstreamResponse) -> RerankProviderResponse {
    let headers: Headers = reply
        .headers
        .iter()
        .map(|(name, value)| (name.as_str().to_owned(), header_text(value)))
        .collect();
    RerankProviderResponse {
        status_code: reply.status,
        headers,
        body: reply.body.clone(),
    }
}
