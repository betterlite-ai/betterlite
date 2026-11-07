use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::rubrik_delivery::clock::Clock;
use betterlite_logging::rubrik_delivery::logger::RubrikLogger;
use betterlite_logging::rubrik_delivery::queue::push_payload;
use betterlite_logging::rubrik_delivery::transport::{
    ClientTier, JsonPost, Transport, TransportFailure, TransportResponse,
};
use betterlite_models::decisions::Json;
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::Level;
use tracing::Subscriber;
use tracing_subscriber::layer::{Context, Layer};

#[derive(Clone, Default, Deserialize)]
#[serde(default)]
pub(super) struct Reply {
    pub(super) status: u16,
    pub(super) body: String,
    pub(super) fail: bool,
    pub(super) during_send: Option<Json>,
}

pub(super) struct ScriptedTransport {
    replies: Mutex<VecDeque<Reply>>,
    calls: Mutex<Vec<Value>>,
    logger: Mutex<Option<Weak<RubrikLogger>>>,
}

impl ScriptedTransport {
    pub(super) fn new(replies: &[Reply]) -> Self {
        Self {
            replies: Mutex::new(replies.iter().map(Reply::clone).collect()),
            calls: Mutex::new(Vec::new()),
            logger: Mutex::new(None),
        }
    }

    pub(super) fn attach(&self, logger: &Arc<RubrikLogger>) {
        let weak = Arc::downgrade(logger);
        let mut slot = self.logger.lock().unwrap_or_else(PoisonError::into_inner);
        *slot = Some(weak);
    }

    pub(super) fn calls(&self) -> Vec<Value> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, request: &JsonPost) {
        let payload: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        let mut headers: Vec<(String, String)> = request
            .headers
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_ascii_lowercase(),
                    value.to_str().unwrap_or_default().to_owned(),
                )
            })
            .collect();
        headers.sort();
        let (tier, timeout) = match request.tier {
            ClientTier::Moderation => (
                "moderation",
                json!({
                    "total": request.timeout.total.as_secs_f64(),
                    "connect": request.timeout.connect.as_secs_f64(),
                }),
            ),
            ClientTier::Logging => ("logging", Value::Null),
        };
        let call = json!({
            "tier": tier,
            "url": request.url,
            "headers": headers,
            "payload": payload,
            "timeout": timeout,
        });
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
    }

    fn next_reply(&self) -> Option<Reply> {
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
    }

    fn push_during_send(&self, reply: &Reply) {
        let Some(queued) = reply.during_send.clone() else {
            return;
        };
        let attached = self
            .logger
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .and_then(Weak::upgrade);
        if let Some(logger) = attached {
            push_payload(&logger, queued);
        }
    }
}

#[async_trait]
impl Transport for ScriptedTransport {
    async fn post_json(&self, request: JsonPost) -> Result<TransportResponse, TransportFailure> {
        self.record(&request);
        let reply = self.next_reply().ok_or_else(scripted_failure)?;
        self.push_during_send(&reply);
        if reply.fail {
            return Err(scripted_failure());
        }
        Ok(TransportResponse {
            status: reply.status,
            body: reply.body.into_bytes(),
        })
    }
}

// A reqwest error cannot be built here, so a missing reply or a scripted failure reports a timeout through the typed HTTP error.
const fn scripted_failure() -> TransportFailure {
    TransportFailure::Request {
        source: TransportError::Rejected(HttpError::Timeout),
    }
}

#[derive(Debug, Default)]
struct ClockValues {
    pending: VecDeque<Duration>,
    last: Duration,
}

pub(super) struct ScriptedClock {
    values: Mutex<ClockValues>,
}

impl ScriptedClock {
    pub(super) fn new(values: &[Duration]) -> Self {
        Self {
            values: Mutex::new(ClockValues {
                pending: values.iter().copied().collect(),
                last: Duration::ZERO,
            }),
        }
    }
}

impl Clock for ScriptedClock {
    fn now_unix(&self) -> Duration {
        let mut values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        match values.pending.pop_front() {
            Some(value) => {
                values.last = value;
                value
            }
            None => values.last,
        }
    }
}

#[derive(Clone, Default)]
pub(super) struct WarningCount(pub(super) Arc<AtomicUsize>);

impl<Inner: Subscriber> Layer<Inner> for WarningCount {
    fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() == Level::WARN {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
}
