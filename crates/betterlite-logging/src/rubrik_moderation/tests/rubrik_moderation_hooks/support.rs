use std::collections::{BTreeMap, VecDeque};
use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::rubrik_delivery::clock::Clock;
use betterlite_logging::rubrik_delivery::config::RubrikInit;
use betterlite_logging::rubrik_delivery::error::{RubrikError, Service};
use betterlite_logging::rubrik_delivery::logger::RubrikLogger;
use betterlite_logging::rubrik_delivery::transport::{
    ClientTier, JsonPost, Timeout, Transport, TransportFailure, TransportResponse,
};
use betterlite_logging::rubrik_moderation::context::LoggingObject;
use betterlite_models::decisions::Json;
use secrecy::SecretString;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

pub(super) const BASE_URL: &str = "https://rubrik.test";
pub(super) const CLOCK_SECONDS: u64 = 1_700_000_000;

#[derive(Clone, Copy, Debug)]
pub(super) struct FixedClock;

impl Clock for FixedClock {
    fn now_unix(&self) -> Duration {
        Duration::from_secs(CLOCK_SECONDS)
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct FarFutureClock;

impl Clock for FarFutureClock {
    fn now_unix(&self) -> Duration {
        Duration::from_secs(u64::MAX)
    }
}

pub(super) fn transient_failure() -> RubrikError {
    RubrikError::Transport {
        service: Service::ResponseModeration,
        source: TransportFailure::Request {
            source: TransportError::Rejected(HttpError::Timeout),
        },
    }
}

#[derive(Clone, Debug)]
pub(super) struct RecordedCall {
    pub(super) tier: ClientTier,
    pub(super) url: String,
    pub(super) payload: Json,
    pub(super) timeout: Timeout,
}

#[derive(Clone, Debug)]
pub(super) enum Outcome {
    Reply { status: u16, body: String },
    Network,
}

#[derive(Clone, Debug)]
pub(super) struct ScriptedReply {
    pub(super) outcome: Outcome,
}

impl ScriptedReply {
    pub(super) fn json(body: &str) -> Self {
        Self {
            outcome: Outcome::Reply {
                status: 200,
                body: body.to_owned(),
            },
        }
    }

    pub(super) fn network() -> Self {
        Self {
            outcome: Outcome::Network,
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct ScriptedTransport {
    replies: Mutex<VecDeque<ScriptedReply>>,
    calls: Mutex<Vec<RecordedCall>>,
}

impl ScriptedTransport {
    pub(super) fn new(replies: Vec<ScriptedReply>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(VecDeque::from(replies)),
            calls: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn calls(&self) -> Vec<RecordedCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl Transport for ScriptedTransport {
    async fn post_json(&self, request: JsonPost) -> Result<TransportResponse, TransportFailure> {
        let payload = serde_json::from_slice::<Json>(&request.body).expect("recorded body is JSON");
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedCall {
                tier: request.tier,
                url: request.url,
                payload,
                timeout: request.timeout,
            });
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .expect("the scripted transport has a reply for every request");
        match reply.outcome {
            Outcome::Reply { status, body } => Ok(TransportResponse {
                status,
                body: body.into_bytes(),
            }),
            Outcome::Network => Err(TransportFailure::Request {
                source: TransportError::Rejected(HttpError::Timeout),
            }),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RecordedEvent {
    pub(super) level: Level,
    pub(super) message: String,
    pub(super) severity: Option<String>,
    pub(super) litellm_call_id: Option<String>,
}

#[derive(Clone, Default)]
pub(super) struct LogCapture(Arc<Mutex<Vec<RecordedEvent>>>);

impl LogCapture {
    pub(super) fn events(&self) -> Vec<RecordedEvent> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn with_level(&self, level: Level) -> Vec<RecordedEvent> {
        self.events()
            .into_iter()
            .filter(|event| event.level == level)
            .collect()
    }
}

#[derive(Default)]
struct EventFields {
    message: String,
    severity: Option<String>,
    litellm_call_id: Option<String>,
}

impl EventFields {
    fn record(&mut self, field: &Field, value: &str) {
        match field.name() {
            "message" => self.message = value.to_owned(),
            "severity" => self.severity = Some(value.to_owned()),
            "litellm_call_id" => self.litellm_call_id = Some(value.to_owned()),
            _ => {}
        }
    }
}

impl Visit for EventFields {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.record(field, value);
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        self.record(field, &format!("{value:?}"));
    }
}

impl<Inner: Subscriber> Layer<Inner> for LogCapture {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut fields = EventFields::default();
        event.record(&mut fields);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedEvent {
                level: *event.metadata().level(),
                message: fields.message,
                severity: fields.severity,
                litellm_call_id: fields.litellm_call_id,
            });
    }
}

pub(super) struct Harness {
    pub(super) logger: Arc<RubrikLogger>,
    pub(super) transport: Arc<ScriptedTransport>,
    pub(super) logs: LogCapture,
    pub(super) _guard: DefaultGuard,
}

impl Harness {
    pub(super) fn new(replies: Vec<ScriptedReply>) -> Self {
        Self::with_clock(replies, Arc::new(FixedClock))
    }

    pub(super) fn with_clock(replies: Vec<ScriptedReply>, clock: Arc<dyn Clock>) -> Self {
        let logs = LogCapture::default();
        let subscriber = tracing_subscriber::registry().with(logs.clone());
        let guard = set_default(subscriber);
        let transport = ScriptedTransport::new(replies);
        let init = RubrikInit {
            api_key: Some(SecretString::from("test-key")),
            api_base: Some(BASE_URL.to_owned()),
            event_hook: None,
            default_on: None,
            guardrail_name: Some("safety".to_owned()),
            timeout: None,
            request_timeout: None,
        };
        let environment = BTreeMap::new();
        let logger = Arc::new(
            RubrikLogger::new(
                &init,
                &environment,
                Arc::<ScriptedTransport>::clone(&transport),
                clock,
            )
            .expect("the logger builds"),
        );
        Self {
            logger,
            transport,
            logs,
            _guard: guard,
        }
    }
}

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("test JSON parses")
}

pub(super) fn logging_object(text: &str) -> Arc<LoggingObject> {
    let Json::Object(details) = json(text) else {
        panic!("the logging object details are an object");
    };
    Arc::new(LoggingObject {
        model_call_details: Mutex::new(details),
    })
}

pub(super) fn field<'value>(value: &'value Json, name: &str) -> &'value Json {
    let Json::Object(fields) = value else {
        panic!("an object is expected");
    };
    fields.get(name).expect("the field is present")
}
