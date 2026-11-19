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
    reply: Mutex<Option<UpstreamResponse>>,
    requests: Mutex<Vec<Recorded>>,
}

impl Sent {
    pub(super) fn new(reply: &Reply) -> Result<Self, Box<HelperError>> {
        let body = match reply.raw.as_ref() {
            Some(text) => text.as_bytes().to_vec(),
            None => serde_json::to_vec(&reply.body).or_value_error()?,
        };
        Ok(Self {
            reply: Mutex::new(Some(UpstreamResponse {
                status: reply.status,
                headers: HeaderMap::new(),
                body,
            })),
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
            .reply
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        ready(reply.ok_or(TransportError::Closed))
    }
}
