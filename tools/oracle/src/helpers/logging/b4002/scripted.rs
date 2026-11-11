use std::collections::{BTreeMap, VecDeque};
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
use serde_json::{Value, json};
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
        let value = request.headers.get(name)?;
        value.to_str().ok()
    };
    let authorization = header("authorization");
    let content_type = header("content-type");
    if request.url.ends_with(TOKEN_PATH) {
        let form: BTreeMap<String, String> =
            form_urlencoded::parse(&request.body).into_owned().collect();
        return json!({
            "authorization": authorization,
            "content_type": content_type,
            "url": request.url,
            "kind": "token",
            "form": form,
        });
    }
    let records: Vec<Value> = serde_json::from_slice(&request.body).unwrap_or_default();
    let entries: Vec<Value> = records.iter().map(entry).collect();
    json!({
        "authorization": authorization,
        "content_type": content_type,
        "url": request.url,
        "kind": "batch",
        "entries": entries,
    })
}

// audit records are kept whole; a model record is told apart by its id and status
fn entry(record: &Value) -> Value {
    if record.get("table_name").is_some() {
        return record.clone();
    }
    json!({ "id": record.get("id"), "status": record.get("status") })
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
