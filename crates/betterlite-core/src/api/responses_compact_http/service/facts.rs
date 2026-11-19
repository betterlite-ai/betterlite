use std::str::from_utf8;
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use serde_json::Number;

use crate::api::vector_store_http_write::error::{
    FailureFacts, HeaderMap, ResponseFacts, header_text,
};

use super::ClientKind;

// What the timeout message needs from the send that just failed
#[derive(Debug, Clone, Copy)]
pub(crate) struct Sent {
    pub(crate) mode: ClientKind,
    pub(crate) requested: Option<Duration>,
    pub(crate) effective: Duration,
    pub(crate) elapsed: Duration,
}

// httpx raises HTTPStatusError after masking the response: the text survives and content-encoding is gone,
// with content-length recomputed for the decoded body
pub(crate) fn status_facts(reply: &UpstreamResponse) -> FailureFacts {
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

// A connect or read timeout becomes litellm.Timeout with status 408 and no headers; any other failure has
// no status of its own, so _handle_error synthesizes a 500 from its text
pub(crate) fn transport_facts(error: &TransportError, sent: &Sent) -> FailureFacts {
    if matches!(
        error,
        TransportError::ConnectTimeout(_) | TransportError::Deadline(_)
    ) {
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

// The sync handler passes the caller's timeout alone, and the async handler passes the timeout it actually
// used; both render the float the way Python's str does, with a missing one as None
pub(crate) fn timeout_message(sent: &Sent) -> String {
    match sent.mode {
        ClientKind::Sync => {
            let passed = sent.requested.map_or_else(
                || String::from("None"),
                |timeout| format!("{:?}", timeout.as_secs_f64()),
            );
            format!("litellm.Timeout: Connection timed out after {passed} seconds.")
        }
        ClientKind::Asynchronous => {
            // Python rounds the float to three decimals; formatting at that precision then dropping the
            // zeros repr() would drop gives the same text without float arithmetic
            let rounded = format!("{:.3}", sent.elapsed.as_secs_f64());
            let trimmed = rounded.trim_end_matches('0');
            let taken = if trimmed.ends_with('.') {
                format!("{trimmed}0")
            } else {
                trimmed.to_owned()
            };
            format!(
                "litellm.Timeout: Connection timed out. Timeout passed={:?}, time taken={taken} seconds",
                sent.effective.as_secs_f64(),
            )
        }
    }
}
