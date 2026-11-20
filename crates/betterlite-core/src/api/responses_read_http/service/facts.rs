use std::str::from_utf8;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use serde_json::Number;

use crate::api::vector_store_http_write::error::{FailureFacts, ResponseFacts, header_map};

// An httpx GET raises the response itself: its status, its decoded text and the headers as received
pub(super) fn status_facts(reply: &UpstreamResponse) -> FailureFacts {
    let text = body_text(&reply.body);
    FailureFacts {
        response_status: Some(i64::from(reply.status)),
        status: Some(Json::Number(Number::from(reply.status))),
        text: Some(text.clone()),
        headers: None,
        response: Some(ResponseFacts {
            status_code: i64::from(reply.status),
            text: Some(text),
            headers: Some(header_map(&reply.headers)),
        }),
        display: String::new(),
    }
}

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

pub(super) fn body_text(body: &[u8]) -> String {
    from_utf8(body).map_or_else(
        |_| format!("{} bytes that are not UTF-8", body.len()),
        ToOwned::to_owned,
    )
}
