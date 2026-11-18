use std::collections::{BTreeMap, VecDeque};
use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_logging::argilla_logging::logger::RandomSource;
use betterlite_logging::rubrik_delivery::clock::Clock;
use betterlite_logging::rubrik_delivery::config::RubrikInit;
use betterlite_logging::rubrik_delivery::logger::RubrikLogger;
use betterlite_logging::rubrik_delivery::transport::{
    JsonPost, Transport, TransportFailure, TransportResponse,
};
use betterlite_logging::rubrik_moderation::context::{
    LoggingObject, LoggingObjectStash, RequestData,
};
use betterlite_logging::rubrik_moderation::error::ModifyResponseException;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

pub(super) const BASE_URL: &str = "https://rubrik.test";
const CLOCK_SECONDS: u64 = 1_700_000_000;

#[derive(Clone, Copy, Debug)]
pub(super) struct FixedClock;

impl Clock for FixedClock {
    fn now_unix(&self) -> Duration {
        Duration::from_secs(CLOCK_SECONDS)
    }
}

#[derive(Debug, Default)]
pub(super) struct ScriptedTransport {
    replies: Mutex<VecDeque<u16>>,
    calls: Mutex<Vec<Json>>,
}

impl ScriptedTransport {
    pub(super) fn new(replies: &[u16]) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(VecDeque::from(replies.to_vec())),
            calls: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn calls(&self) -> Vec<Json> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl Transport for ScriptedTransport {
    async fn post_json(&self, request: JsonPost) -> Result<TransportResponse, TransportFailure> {
        let payload =
            serde_json::from_slice::<Json>(&request.body).expect("the batch body is JSON");
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(payload);
        let status = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .expect("a reply for every batch");
        Ok(TransportResponse {
            status,
            body: Vec::new(),
        })
    }
}

#[derive(Debug)]
pub(super) struct ScriptedRandom {
    samples: Mutex<VecDeque<f64>>,
}

impl ScriptedRandom {
    pub(super) fn new(samples: &[f64]) -> Self {
        Self {
            samples: Mutex::new(VecDeque::from(samples.to_vec())),
        }
    }
}

impl RandomSource for ScriptedRandom {
    fn sample(&self) -> f64 {
        self.samples
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .expect("a scripted sample for every draw")
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct UnusedRandom;

impl RandomSource for UnusedRandom {
    fn sample(&self) -> f64 {
        panic!("this path must not sample")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RecordedEvent {
    pub(super) level: Level,
    pub(super) message: String,
    pub(super) event: Option<String>,
    pub(super) litellm_call_id: Option<String>,
    pub(super) model: Option<String>,
    pub(super) user_id: Option<String>,
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
    event: Option<String>,
    litellm_call_id: Option<String>,
    model: Option<String>,
    user_id: Option<String>,
}

impl EventFields {
    fn record(&mut self, field: &Field, value: &str) {
        match field.name() {
            "message" => self.message = value.to_owned(),
            "event" => self.event = Some(value.to_owned()),
            "litellm_call_id" => self.litellm_call_id = Some(value.to_owned()),
            "model" => self.model = Some(value.to_owned()),
            "user_id" => self.user_id = Some(value.to_owned()),
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
                event: fields.event,
                litellm_call_id: fields.litellm_call_id,
                model: fields.model,
                user_id: fields.user_id,
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
    pub(super) fn new(environment: &[(&str, &str)], replies: &[u16]) -> Self {
        let logs = LogCapture::default();
        let subscriber = tracing_subscriber::registry().with(logs.clone());
        let guard = set_default(subscriber);
        let transport = ScriptedTransport::new(replies);
        let init = RubrikInit {
            api_key: Some(SecretString::from("test-key")),
            api_base: Some(BASE_URL.to_owned()),
            event_hook: None,
            default_on: None,
            guardrail_name: None,
            timeout: None,
            request_timeout: None,
        };
        let environment = environment
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect::<BTreeMap<_, _>>();
        let logger = Arc::new(
            RubrikLogger::new(
                &init,
                &environment,
                Arc::<ScriptedTransport>::clone(&transport),
                Arc::new(FixedClock),
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

pub(super) fn details(text: &str) -> IndexMap<String, Json> {
    let Json::Object(fields) = json(text) else {
        panic!("the call details are an object");
    };
    fields
}

pub(super) fn field<'value>(value: &'value Json, name: &str) -> &'value Json {
    let Json::Object(fields) = value else {
        panic!("an object is expected");
    };
    fields.get(name).expect("the field is present")
}

pub(super) fn text(value: &Json) -> &str {
    let Json::Text(content) = value else {
        panic!("a string is expected");
    };
    content
}

pub(super) fn object_keys(value: &Json) -> Vec<&str> {
    let Json::Object(fields) = value else {
        panic!("an object is expected");
    };
    fields.keys().map(String::as_str).collect()
}

pub(super) fn logging_object(text: &str) -> Arc<LoggingObject> {
    Arc::new(LoggingObject {
        model_call_details: Mutex::new(details(text)),
    })
}

pub(super) fn exception(message: &str, guardrail_name: &str) -> ModifyResponseException {
    ModifyResponseException {
        message: message.to_owned(),
        model: "gpt-4o".to_owned(),
        request_data: Box::default(),
        guardrail_name: guardrail_name.to_owned(),
    }
}

pub(super) fn request_data(stash: LoggingObjectStash) -> RequestData {
    RequestData {
        fields: details(r#"{"litellm_call_id":"call-1","model":"gpt-4o"}"#),
        logging_obj: stash,
    }
}
