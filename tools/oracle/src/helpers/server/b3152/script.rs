use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use axum::http::header::{ACCEPT, AUTHORIZATION};
use axum::http::{HeaderMap, HeaderName, HeaderValue};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_server::roi_github::transport::Transport;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::task::yield_now;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ReplyInput {
    #[serde(default)]
    pub status: u16,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    #[serde(default)]
    pub body: Option<Value>,
    #[serde(default)]
    pub raw: Option<String>,
    #[serde(default)]
    pub fail: bool,
}

#[derive(Clone, Default)]
pub(crate) struct Script {
    replies: Arc<Mutex<VecDeque<ReplyInput>>>,
    seen: Arc<Mutex<Vec<Value>>>,
}

impl Script {
    pub(crate) fn new(replies: Vec<ReplyInput>) -> Self {
        Self {
            replies: Arc::new(Mutex::new(VecDeque::from(replies))),
            seen: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(crate) fn seen(&self) -> Vec<Value> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Transport for Script {
    async fn send(&self, request: OutboundRequest) -> Result<UpstreamResponse, TransportError> {
        yield_now().await;
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(recorded(&request));
        let next = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        let Some(reply) = next else {
            return Err(TransportError::BodyTooLarge { limit: 1 });
        };
        if reply.fail {
            return Err(TransportError::BodyTooLarge { limit: 1 });
        }
        answer(reply)
    }
}

fn recorded(request: &OutboundRequest) -> Value {
    let header = |name: &str| {
        request
            .headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };
    json!({
        "method": request.method.as_str(),
        "url": request.url.to_string(),
        "query": request.query,
        "accept": header(ACCEPT.as_str()),
        "authorization": header(AUTHORIZATION.as_str()),
    })
}

fn answer(reply: ReplyInput) -> Result<UpstreamResponse, TransportError> {
    let mut headers = HeaderMap::new();
    for (name, raw) in reply.headers {
        let field = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_error| TransportError::BodyTooLarge { limit: 1 })?;
        let value = HeaderValue::from_str(&raw)
            .map_err(|_error| TransportError::BodyTooLarge { limit: 1 })?;
        headers.append(field, value);
    }
    let body = match (reply.raw, reply.body) {
        (Some(raw), _) => raw.into_bytes(),
        (None, Some(value)) => serde_json::to_vec(&value).unwrap_or_default(),
        (None, None) => Vec::new(),
    };
    Ok(UpstreamResponse {
        status: reply.status,
        headers,
        body,
    })
}
