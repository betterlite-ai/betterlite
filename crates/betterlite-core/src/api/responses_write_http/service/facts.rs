use std::str::from_utf8;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::response_events::RawResponse;
use indexmap::IndexMap;

use crate::api::vector_store_http_write::error::{FailureFacts, header_text};

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
