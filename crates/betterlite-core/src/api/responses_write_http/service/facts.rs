use std::str::from_utf8;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use betterlite_openai::response_events::RawResponse;
use indexmap::IndexMap;
use serde_json::Number;

use crate::api::vector_store_http_write::error::{
    FailureFacts, HeaderMap, ResponseFacts, header_text,
};

use super::super::error::WriteError;

// The successful reply in B508's form, whose header and text shapes the B509 transforms take
pub(super) struct ReplyView {
    status: u16,
    headers: IndexMap<String, String>,
    text: String,
}

impl ReplyView {
    pub(super) fn new(reply: &UpstreamResponse) -> Result<Self, WriteError> {
        let text = from_utf8(&reply.body)
            .map_err(WriteError::ReplyText)?
            .to_owned();
        let headers = reply
            .headers
            .iter()
            .map(|(name, value)| (name.as_str().to_owned(), header_text(value)))
            .collect();
        Ok(Self {
            status: reply.status,
            headers,
            text,
        })
    }

    pub(super) fn raw(&self) -> RawResponse<'_> {
        RawResponse {
            status: self.status,
            headers: &self.headers,
            text: &self.text,
        }
    }
}

// LiteLLM's HTTPStatusError carries the answered status, its text and its headers; B2402's facts shape is the same here
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

// Both handler families catch every send exception, so a timeout is no special case: it becomes the synthesized 500
pub(super) fn transport_facts(error: &TransportError) -> FailureFacts {
    FailureFacts {
        response_status: None,
        status: None,
        text: None,
        headers: None,
        response: None,
        display: error.to_string(),
    }
}
