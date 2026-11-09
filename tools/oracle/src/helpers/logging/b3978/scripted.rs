use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::rubrik_delivery::clock::Clock;
use betterlite_logging::rubrik_delivery::transport::{
    ClientTier, JsonPost, Transport, TransportFailure, TransportResponse,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct Reply {
    pub(super) status: u16,
    pub(super) body: String,
    pub(super) fail: bool,
}

impl Default for Reply {
    fn default() -> Self {
        Self {
            status: 200,
            body: String::new(),
            fail: false,
        }
    }
}

pub(super) struct ScriptedTransport {
    replies: Mutex<VecDeque<Reply>>,
    calls: Mutex<Vec<Value>>,
}

impl ScriptedTransport {
    pub(super) fn new(replies: &[Reply]) -> Self {
        Self {
            replies: Mutex::new(replies.iter().map(Clone::clone).collect()),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn calls(&self) -> Vec<Value> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, request: &JsonPost) {
        let payload: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
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
        let tier = match request.tier {
            ClientTier::Moderation => "moderation",
            ClientTier::Logging => "logging",
        };
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({
                "tier": tier,
                "url": request.url,
                "headers": headers,
                "payload": payload,
                "timeout": {
                    "total": request.timeout.total.as_secs_f64(),
                    "connect": request.timeout.connect.as_secs_f64(),
                },
            }));
    }
}

#[async_trait]
impl Transport for ScriptedTransport {
    async fn post_json(&self, request: JsonPost) -> Result<TransportResponse, TransportFailure> {
        self.record(&request);
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .ok_or_else(scripted_failure)?;
        if reply.fail {
            return Err(scripted_failure());
        }
        Ok(TransportResponse {
            status: reply.status,
            body: reply.body.into_bytes(),
        })
    }
}

// A reqwest error cannot be built here, so a scripted failure reports a timeout through the typed HTTP error.
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

#[derive(Debug)]
struct LogEvent {
    level: Level,
    message: String,
    severity: Option<String>,
    litellm_call_id: Option<String>,
    label: Option<String>,
}

#[derive(Clone, Default)]
pub(super) struct LogCapture {
    events: Arc<Mutex<Vec<LogEvent>>>,
    critical: Arc<AtomicUsize>,
    errors: Arc<AtomicUsize>,
}

impl LogCapture {
    pub(super) fn report(&self) -> Value {
        let events = self
            .events
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut warnings = Vec::new();
        let mut call_ids = Vec::new();
        let mut labels = Vec::new();
        for event in events.iter() {
            if event.level == Level::WARN {
                warnings.push(event.message.clone());
            }
            if let Some(call_id) = &event.litellm_call_id {
                call_ids.push(call_id.clone());
            }
            if let Some(label) = &event.label {
                labels.push(label.clone());
            }
        }
        drop(events);
        json!({
            "warnings": warnings,
            "critical": self.critical.load(Ordering::SeqCst),
            "errors": self.errors.load(Ordering::SeqCst),
            "error_call_ids": call_ids,
            "error_labels": labels,
        })
    }
}

#[derive(Default)]
struct EventFields {
    message: Option<String>,
    severity: Option<String>,
    litellm_call_id: Option<String>,
    label: Option<String>,
}

impl EventFields {
    fn record(&mut self, field: &Field, value: &str) {
        match field.name() {
            "message" => self.message = Some(value.to_owned()),
            "severity" => self.severity = Some(value.to_owned()),
            "litellm_call_id" => self.litellm_call_id = Some(value.to_owned()),
            "label" => self.label = Some(value.to_owned()),
            _ => {}
        }
    }
}

impl Visit for EventFields {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.record(field, value);
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.record(field, &format!("{value:?}"));
    }
}

impl<Inner: Subscriber> Layer<Inner> for LogCapture {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if !event
            .metadata()
            .target()
            .starts_with("betterlite_logging::rubrik_moderation")
        {
            return;
        }
        let mut fields = EventFields::default();
        event.record(&mut fields);
        let recorded = LogEvent {
            level: *event.metadata().level(),
            message: fields.message.unwrap_or_default(),
            severity: fields.severity,
            litellm_call_id: fields.litellm_call_id,
            label: fields.label,
        };
        match recorded.level {
            Level::ERROR => {
                if recorded.severity.as_deref() == Some("critical") {
                    self.critical.fetch_add(1, Ordering::SeqCst);
                } else {
                    self.errors.fetch_add(1, Ordering::SeqCst);
                }
            }
            Level::WARN => {}
            Level::INFO | Level::DEBUG | Level::TRACE => return,
        }
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(recorded);
    }
}
