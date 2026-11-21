use std::collections::VecDeque;
use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};

use betterlite_bedrock_guardrails::bedrock_post::GuardrailTransport;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use reqwest::header::HeaderMap;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Reply {
    pub(crate) status: u16,
    pub(crate) body: Value,
}

// Answers each ApplyGuardrail call from the case script and keeps the JSON body it sent.
pub(crate) struct Sent {
    replies: Mutex<VecDeque<Reply>>,
    bodies: Mutex<Vec<Value>>,
}

impl Sent {
    pub(crate) fn new(replies: Vec<Reply>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            bodies: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn bodies(&self) -> Vec<Value> {
        self.bodies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

fn response(reply: Reply) -> UpstreamResponse {
    UpstreamResponse {
        status: reply.status,
        headers: HeaderMap::new(),
        body: serde_json::to_vec(&reply.body).unwrap_or_default(),
    }
}

impl GuardrailTransport for Sent {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        if let RequestBody::Json(bytes) = &request.body {
            self.bodies
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(serde_json::from_slice(bytes).unwrap_or(Value::Null));
        }
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        ready(reply.map(response).ok_or(TransportError::Closed))
    }
}
