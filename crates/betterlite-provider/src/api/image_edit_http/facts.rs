use std::str::from_utf8;
use std::time::Duration;

use betterlite_core::api::vector_store_http_write::error::{
    FailureFacts, HeaderMap, ResponseFacts, header_text,
};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{ImageHeaders, ImageHttpResponse};
use indexmap::IndexMap;
use serde_json::Number;

use super::service::ClientKind;

// The shared converter's input plus the header map the provider's error hook receives
pub(super) struct Failure {
    pub(super) facts: FailureFacts,
    pub(super) headers: ImageHeaders,
}

// A send that failed before any reply: the timeouts the HTTP handlers catch and build themselves, and everything else
#[derive(Debug, Clone, Copy)]
pub(super) enum SendError<'error> {
    TimedOut,
    Failed(&'error TransportError),
}

// What the timeout message needs from the send that just failed
#[derive(Debug, Clone, Copy)]
pub(super) struct Sent {
    pub(super) mode: ClientKind,
    pub(super) timeout: Duration,
    pub(super) elapsed: Duration,
}

// A status of 400 or more: the masked response's status, body text and headers, with content-length recomputed from the decoded body
pub(super) fn reply_failure(reply: &UpstreamResponse) -> Failure {
    let headers = answer_headers(reply);
    let text = from_utf8(&reply.body).map_or_else(
        |_| format!("{} bytes that are not UTF-8", reply.body.len()),
        ToOwned::to_owned,
    );
    let recorded: HeaderMap = headers
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
        .collect();
    Failure {
        facts: FailureFacts {
            response_status: Some(i64::from(reply.status)),
            status: Some(Json::Number(Number::from(reply.status))),
            text: Some(text.clone()),
            headers: Some(recorded.clone()),
            response: Some(ResponseFacts {
                status_code: i64::from(reply.status),
                text: Some(text),
                headers: Some(recorded),
            }),
            display: String::new(),
        },
        headers,
    }
}

// The two exception shapes llm_http_handler.py reads when the send itself raised
pub(super) fn send_failure(error: SendError<'_>, sent: Sent) -> Failure {
    let facts = match error {
        SendError::TimedOut => FailureFacts {
            response_status: None,
            status: Some(Json::Number(Number::from(408_u16))),
            text: Some(timeout_message(sent)),
            headers: Some(HeaderMap::new()),
            response: None,
            display: String::new(),
        },
        SendError::Failed(error) => FailureFacts {
            response_status: None,
            status: None,
            text: None,
            headers: None,
            response: None,
            display: error.to_string(),
        },
    };
    Failure {
        facts,
        headers: ImageHeaders::new(),
    }
}

// httpx rebuilds the masked response from the decoded body, so only a non-empty one carries a content-length
fn answer_headers(reply: &UpstreamResponse) -> ImageHeaders {
    let mut headers: ImageHeaders = provider_headers(&reply.headers).into_iter().collect();
    headers
        .retain(|name, _| name.as_str() != "content-encoding" && name.as_str() != "content-length");
    if !reply.body.is_empty() {
        headers.insert(String::from("content-length"), reply.body.len().to_string());
    }
    headers
}

fn timeout_message(sent: Sent) -> String {
    let timeout = seconds(sent.timeout);
    match sent.mode {
        ClientKind::Sync => {
            format!("litellm.Timeout: Connection timed out after {timeout} seconds.")
        }
        ClientKind::Asynchronous => format!(
            "litellm.Timeout: Connection timed out. Timeout passed={timeout}, time taken={} seconds",
            seconds(sent.elapsed)
        ),
    }
}

// Python prints the timeout as a float, so whole seconds keep a trailing .0
fn seconds(duration: Duration) -> String {
    let fraction = duration.subsec_micros();
    match fraction {
        0 => format!("{}.0", duration.as_secs()),
        _ => format!("{}.{fraction:06}", duration.as_secs())
            .trim_end_matches('0')
            .to_owned(),
    }
}

// The headers a response carries as a mapping: names lowercased by the HTTP layer, repeats joined
pub(super) fn provider_headers(headers: &http::HeaderMap) -> IndexMap<String, String> {
    let mut provider = IndexMap::new();
    for name in headers.keys() {
        let joined: Vec<String> = headers.get_all(name).iter().map(header_text).collect();
        provider.insert(name.as_str().to_owned(), joined.join(", "));
    }
    provider
}

pub(super) fn image_reply(reply: &UpstreamResponse) -> ImageHttpResponse {
    ImageHttpResponse {
        status_code: reply.status,
        headers: provider_headers(&reply.headers).into_iter().collect(),
        body: reply.body.clone(),
    }
}
