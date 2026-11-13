use std::collections::{BTreeMap, VecDeque};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_logging::datadog_logging::clock::Clock;
use betterlite_logging::datadog_logging::transport::{
    PostRequest, Transport, TransportFailure, TransportResponse,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::normalize;

#[derive(Clone, Deserialize, Default)]
#[serde(default)]
pub(super) struct Reply {
    status: u16,
    body: String,
}

pub(super) struct ScriptedTransport {
    replies: Mutex<VecDeque<Reply>>,
    calls: Mutex<Vec<Value>>,
}

impl ScriptedTransport {
    pub(super) fn new(replies: &[Reply]) -> Self {
        Self {
            replies: Mutex::new(replies.iter().cloned().collect()),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn calls(&self) -> Vec<Value> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl Transport for ScriptedTransport {
    async fn post(&self, request: PostRequest) -> Result<TransportResponse, TransportFailure> {
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_default();
        let headers: BTreeMap<String, String> = request
            .headers
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_ascii_lowercase(),
                    value.to_str().unwrap_or_default().to_owned(),
                )
            })
            .collect();
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({
                "url": request.url,
                "headers": headers,
                "body": normalize::body(&request.body, &request.headers),
            }));
        Ok(TransportResponse {
            status: reply.status,
            body: reply.body.into_bytes(),
        })
    }
}

pub(super) struct FixedClock(pub(super) Duration);

impl Clock for FixedClock {
    fn now_unix(&self) -> Duration {
        self.0
    }
}
