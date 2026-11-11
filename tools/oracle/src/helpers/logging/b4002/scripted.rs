use std::collections::VecDeque;
use std::future::pending;
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_logging::azure_sentinel_logging::logger::AzureSentinelLogger;
use betterlite_logging::datadog_logging::clock::Clock;
use betterlite_logging::datadog_logging::transport::{
    PostRequest, Transport, TransportFailure, TransportResponse,
};
use serde::Deserialize;
use serde_json::{Map, Value};
use url::form_urlencoded;

use super::records::{audit_payload, model_payload};

const TOKEN_PATH: &str = "/oauth2/v2.0/token";

#[derive(Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct Arrivals {
    model: Vec<String>,
    audit: Vec<String>,
}

#[derive(Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct Reply {
    status: u16,
    body: String,
    network: bool,
    cancel: bool,
    arrive: Arrivals,
}

pub(super) struct ScriptedTransport {
    replies: Mutex<VecDeque<Reply>>,
    calls: Mutex<Vec<Value>>,
    logger: Mutex<Weak<AzureSentinelLogger>>,
}

impl ScriptedTransport {
    pub(super) fn new(replies: &[Reply]) -> Self {
        Self {
            replies: Mutex::new(replies.iter().cloned().collect()),
            calls: Mutex::new(Vec::new()),
            logger: Mutex::new(Weak::new()),
        }
    }

    pub(super) fn attach(&self, logger: &Arc<AzureSentinelLogger>) {
        let weak = Arc::downgrade(logger);
        let mut slot = self.logger.lock().unwrap_or_else(PoisonError::into_inner);
        *slot = weak;
    }

    pub(super) fn calls(&self) -> Vec<Value> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    // records that reach the queue while a request is in flight
    fn arrive(&self, arrivals: &Arrivals) -> Result<(), serde_json::Error> {
        let attached = self
            .logger
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .upgrade();
        let Some(logger) = attached else {
            return Ok(());
        };
        for id in &arrivals.model {
            let _enqueued = logger.model.queue.enqueue(model_payload(id, false, 0)?);
        }
        for id in &arrivals.audit {
            let _enqueued = logger.audit.queue.enqueue(audit_payload(id));
        }
        Ok(())
    }
}

const NETWORK_FAILURE: TransportFailure = TransportFailure::Url(url::ParseError::EmptyHost);

fn call_json(request: &PostRequest) -> Value {
    let header = |name: &str| {
        request
            .headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };
    let mut call = Map::new();
    call.insert(
        "authorization".to_owned(),
        Value::from(header("authorization")),
    );
    call.insert(
        "content_type".to_owned(),
        Value::from(header("content-type")),
    );
    call.insert("url".to_owned(), Value::from(request.url.as_str()));
    if request.url.ends_with(TOKEN_PATH) {
        let form: Map<String, Value> = form_urlencoded::parse(&request.body)
            .map(|(name, value)| (name.into_owned(), Value::from(value.into_owned())))
            .collect();
        call.insert("kind".to_owned(), Value::from("token"));
        call.insert("form".to_owned(), Value::Object(form));
    } else {
        let records: Vec<Value> = serde_json::from_slice(&request.body).unwrap_or_default();
        let entries: Vec<Value> = records.iter().map(entry).collect();
        call.insert("kind".to_owned(), Value::from("batch"));
        call.insert("entries".to_owned(), Value::Array(entries));
    }
    Value::Object(call)
}

// audit records are kept whole; a model record is told apart by its id and status
fn entry(record: &Value) -> Value {
    if record.get("table_name").is_some() {
        return record.clone();
    }
    let mut kept = Map::new();
    kept.insert(
        "id".to_owned(),
        record.get("id").cloned().unwrap_or(Value::Null),
    );
    kept.insert(
        "status".to_owned(),
        record.get("status").cloned().unwrap_or(Value::Null),
    );
    Value::Object(kept)
}

#[async_trait]
impl Transport for ScriptedTransport {
    async fn post(&self, request: PostRequest) -> Result<TransportResponse, TransportFailure> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call_json(&request));
        let next = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        let Some(reply) = next else {
            return Err(NETWORK_FAILURE);
        };
        if self.arrive(&reply.arrive).is_err() {
            return Err(NETWORK_FAILURE);
        }
        if reply.cancel {
            return pending().await;
        }
        if reply.network {
            return Err(NETWORK_FAILURE);
        }
        Ok(TransportResponse {
            status: reply.status,
            body: reply.body.into_bytes(),
        })
    }
}

pub(super) struct ManualClock(pub(super) Mutex<Duration>);

impl Clock for ManualClock {
    fn now_unix(&self) -> Duration {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
