use std::collections::VecDeque;
use std::io::Read as _;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::datadog_logging::clock::Clock;
use betterlite_logging::datadog_logging::transport::{
    PostRequest, Transport, TransportFailure, TransportResponse,
};
use flate2::read::GzDecoder;
use serde_json::{Map, Value, json};

use super::ReplyInput;

const GZIP_ENCODING: &str = "gzip";

pub(super) struct ScriptedTransport {
    replies: Mutex<VecDeque<u16>>,
    calls: Mutex<Vec<Value>>,
}

impl ScriptedTransport {
    pub(super) fn new(replies: &[ReplyInput]) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(replies.iter().map(|reply| reply.status).collect()),
            calls: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn calls(&self) -> Vec<Value> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

fn gzipped(request: &PostRequest) -> bool {
    request
        .headers
        .get("content-encoding")
        .is_some_and(|value| value == GZIP_ENCODING)
}

fn decoded(request: &PostRequest) -> Vec<u8> {
    if !gzipped(request) {
        return request.body.clone();
    }
    let mut decoded = Vec::new();
    let _read = GzDecoder::new(request.body.as_slice()).read_to_end(&mut decoded);
    decoded
}

fn record(request: &PostRequest) -> Value {
    let mut headers = Map::new();
    for (name, value) in &request.headers {
        headers.insert(
            name.as_str().to_ascii_lowercase(),
            json!(value.to_str().unwrap_or_default()),
        );
    }
    let body: Value = serde_json::from_slice(&decoded(request)).unwrap_or(Value::Null);
    json!({
        "url": request.url,
        "headers": Value::Object(headers),
        "body": body,
        "gzip": gzipped(request),
    })
}

#[async_trait]
impl Transport for ScriptedTransport {
    async fn post(&self, request: PostRequest) -> Result<TransportResponse, TransportFailure> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(record(&request));
        let status = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .ok_or_else(|| {
                TransportFailure::Request(TransportError::Rejected(HttpError::Timeout))
            })?;
        Ok(TransportResponse {
            status,
            body: Vec::new(),
        })
    }
}

#[derive(Clone, Copy)]
pub(super) struct FixedClock(pub(super) Duration);

impl Clock for FixedClock {
    fn now_unix(&self) -> Duration {
        self.0
    }
}
