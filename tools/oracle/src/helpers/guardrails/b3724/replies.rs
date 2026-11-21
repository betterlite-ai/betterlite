use std::collections::VecDeque;
use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};

use betterlite_bedrock_guardrails::bedrock_post::GuardrailTransport;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use reqwest::header::HeaderMap;
use serde::Serialize;
use serde_json::Value;

use super::Reply;

#[derive(Debug, Clone, Serialize)]
pub(super) struct Recorded {
    url: String,
    authorization: Option<String>,
    content_type: Option<String>,
    body: Value,
}

pub(super) struct Sent {
    replies: Mutex<VecDeque<UpstreamResponse>>,
    requests: Mutex<Vec<Recorded>>,
}

fn response_of(reply: &Reply) -> Result<UpstreamResponse, Box<HelperError>> {
    let body = match reply.raw.as_ref() {
        Some(text) => text.as_bytes().to_vec(),
        None => serde_json::to_vec(&reply.body).or_value_error()?,
    };
    Ok(UpstreamResponse {
        status: reply.status,
        headers: HeaderMap::new(),
        body,
    })
}

impl Sent {
    pub(super) fn new(replies: &[Reply]) -> Result<Self, Box<HelperError>> {
        let queue: Result<VecDeque<UpstreamResponse>, Box<HelperError>> =
            replies.iter().map(response_of).collect();
        Ok(Self {
            replies: Mutex::new(queue?),
            requests: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn requests(&self) -> Vec<Recorded> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, request: &OutboundRequest) {
        let recorded = Recorded {
            url: request.url.to_string(),
            authorization: header(&request.headers, "authorization"),
            content_type: header(&request.headers, "content-type"),
            body: recorded_body(&request.body),
        };
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(recorded);
    }
}

fn header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn recorded_body(body: &RequestBody) -> Value {
    match body {
        RequestBody::Json(bytes) => serde_json::from_slice(bytes).unwrap_or(Value::Null),
        RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => {
            Value::Null
        }
    }
}

impl GuardrailTransport for Sent {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.record(&request);
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        ready(reply.ok_or(TransportError::Closed))
    }
}
