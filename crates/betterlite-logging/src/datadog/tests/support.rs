use std::collections::BTreeMap;
use std::fmt;
use std::io::Read as _;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_error::normalize::Normalizer;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};
use flate2::read::GzDecoder;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::datadog::apm::{Span, Tracer};
use crate::datadog::payloads::legacy::{LegacyLoggingEvent, LegacyLoggingKwargs};
use crate::datadog::payloads::status::DataDogStatus;
use crate::datadog::payloads::types::DatadogPayload;
use crate::datadog::tagging::Environment;
use crate::datadog_logging::clock::Clock;
use crate::datadog_logging::logger::{DatadogDeps, DatadogLogger};
use crate::datadog_logging::settings::{DatadogInit, DatadogInitParams};
use crate::datadog_logging::transport::{
    PostRequest, Transport, TransportFailure, TransportResponse,
};
use crate::fields::payload::StandardLoggingPayload;
use crate::logging_metadata::error_info::ExceptionLogging;
use crate::logging_metadata::exception::ExceptionFacts;
use crate::logging_service_events::types::{ServiceLoggerPayload, ServiceType};

const STANDARD_PAYLOAD: &str = r#"{
    "id": "chatcmpl-1",
    "trace_id": "trace-1",
    "session_id": "session-1",
    "call_type": "completion",
    "response_cost": 0.5,
    "status": "success",
    "status_fields": {"llm_api_status": "success", "guardrail_status": "not_run"},
    "total_tokens": 5,
    "prompt_tokens": 2,
    "completion_tokens": 3,
    "startTime": 1704067200.0,
    "endTime": 1704067201.5,
    "completionStartTime": 1704067200.5,
    "response_time": 1.0,
    "model_map_information": {"model_map_key": "", "model_map_value": null},
    "model": "gpt-4o",
    "api_base": "https://api.openai.com",
    "metadata": {},
    "saved_cache_cost": 0.0,
    "request_tags": [],
    "model_parameters": {},
    "hidden_params": {}
}"#;

const TAGGED_PAYLOAD: &str = r#"{
    "id": "chatcmpl-1",
    "trace_id": "trace-1",
    "session_id": "session-1",
    "call_type": "completion",
    "response_cost": 0.5,
    "status": "success",
    "status_fields": {"llm_api_status": "success", "guardrail_status": "not_run"},
    "total_tokens": 5,
    "prompt_tokens": 2,
    "completion_tokens": 3,
    "startTime": 1704067200.0,
    "endTime": 1704067201.5,
    "completionStartTime": 1704067200.5,
    "response_time": 1.0,
    "model_map_information": {"model_map_key": "", "model_map_value": null},
    "model": "gpt-4o",
    "api_base": "https://api.openai.com",
    "metadata": {"user_api_key_team_alias": "Team One", "team_id": "ignored-team"},
    "saved_cache_cost": 0.0,
    "request_tags": ["Prod A", 7],
    "model_parameters": {},
    "hidden_params": {}
}"#;

pub(crate) fn lookup(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| (String::from(*name), String::from(*value)))
        .collect()
}

pub(crate) fn environment(pairs: &[(&str, &str)]) -> Environment {
    Environment::from_lookup(&lookup(pairs)).unwrap()
}

pub(crate) fn default_environment() -> Environment {
    Environment::from_lookup(&BTreeMap::new()).unwrap()
}

pub(crate) fn standard_payload() -> StandardLoggingPayload {
    serde_json::from_str(STANDARD_PAYLOAD).unwrap()
}

pub(crate) fn tagged_payload() -> StandardLoggingPayload {
    serde_json::from_str(TAGGED_PAYLOAD).unwrap()
}

pub(crate) fn message(payload: &DatadogPayload) -> Json {
    serde_json::from_str(&payload.message).unwrap()
}

pub(crate) fn json(encoded: &str) -> Json {
    serde_json::from_str(encoded).unwrap()
}

pub(crate) fn field<'json>(value: &'json Json, name: &str) -> &'json Json {
    match value {
        Json::Object(fields) => fields.get(name).unwrap_or_else(|| panic!("missing {name}")),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("expected a JSON object")
        }
    }
}

pub(crate) fn has_key(value: &Json, name: &str) -> bool {
    match value {
        Json::Object(fields) => fields.contains_key(name),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => false,
    }
}

pub(crate) fn moment(encoded: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(encoded).unwrap()
}

pub(crate) fn default_kwargs() -> LegacyLoggingKwargs {
    serde_json::from_str("{}").unwrap()
}

pub(crate) fn nested_json(depth: usize) -> Json {
    let mut value = Json::Text("deep".to_owned());
    for _ in 0..depth {
        value = Json::List(vec![value]);
    }
    value
}

pub(crate) fn legacy_event<'event>(
    kwargs: &'event LegacyLoggingKwargs,
    response_obj: &'event Json,
) -> LegacyLoggingEvent<'event> {
    LegacyLoggingEvent {
        kwargs,
        response_obj,
        start_time: moment("2024-01-01T00:00:00+00:00"),
        end_time: moment("2024-01-01T00:00:01.500+00:00"),
        status: DataDogStatus::Info,
        generated_id: "generated-id",
    }
}

pub(crate) fn text(value: &Json) -> &str {
    match value {
        Json::Text(encoded) => encoded,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            panic!("expected JSON text, got {value:?}")
        }
    }
}

pub(crate) fn exception(encoded: &str) -> ExceptionFacts {
    serde_json::from_str(encoded).expect("the exception facts decode")
}

pub(crate) fn normalizer() -> Normalizer {
    Normalizer::new().expect("the normalizer patterns compile")
}

pub(crate) fn exception_logging<'input>(
    exception: Option<&'input ExceptionFacts>,
    normalizer: &'input Normalizer,
) -> ExceptionLogging<'input> {
    ExceptionLogging {
        exception,
        log_client_error_tracebacks: false,
        expected_client_error: false,
        redactor: None,
        normalizer,
    }
}

pub(crate) fn service_payload(
    service: ServiceType,
    is_error: bool,
    error: Option<&str>,
    duration: f64,
) -> ServiceLoggerPayload {
    ServiceLoggerPayload {
        is_error,
        error: error.map(str::to_owned),
        service,
        duration,
        call_type: "acompletion".to_owned(),
        caller: None,
        event_metadata: None,
    }
}

pub(crate) struct TestSpan {
    pub(crate) trace_id: Option<u128>,
    pub(crate) span_id: Option<u64>,
}

impl Span for TestSpan {
    fn trace_id(&self) -> Option<u128> {
        self.trace_id
    }

    fn span_id(&self) -> Option<u64> {
        self.span_id
    }
}

// the spans are Sync so the tracer can cross an await point, as the server's `Arc<dyn Tracer + Send + Sync>` requires
pub(crate) struct TestTracer<'span>(
    pub(crate) Option<&'span (dyn Span + Sync)>,
    pub(crate) Option<&'span (dyn Span + Sync)>,
);

impl Tracer for TestTracer<'_> {
    fn current_span(&self) -> Option<&dyn Span> {
        let span = self.0?;
        Some(span)
    }

    fn current_root_span(&self) -> Option<&dyn Span> {
        let span = self.1?;
        Some(span)
    }
}

pub(crate) struct FixedClock(pub(crate) Duration);

impl Clock for FixedClock {
    fn now_unix(&self) -> Duration {
        self.0
    }
}

// The two answers the intake gives a test: a 202 the queue drains on, or a dead connection it requeues on
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Intake {
    Accepted,
    Unreachable,
}

pub(crate) struct RecordingTransport {
    intake: Intake,
    bodies: Mutex<Vec<Json>>,
}

impl RecordingTransport {
    fn new(intake: Intake) -> Arc<Self> {
        Arc::new(Self {
            intake,
            bodies: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn post_count(&self) -> usize {
        self.bodies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    pub(crate) fn posted_events(&self) -> Vec<Json> {
        let bodies = self.bodies.lock().unwrap_or_else(PoisonError::into_inner);
        let mut events = Vec::new();
        for body in bodies.as_slice() {
            let Json::List(items) = body else {
                panic!("a flush posts a JSON array of payloads");
            };
            events.extend(items.iter().cloned());
        }
        drop(bodies);
        events
    }
}

#[async_trait]
impl Transport for RecordingTransport {
    async fn post(&self, request: PostRequest) -> Result<TransportResponse, TransportFailure> {
        let body = decoded_body(&request);
        self.bodies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(body);
        match self.intake {
            Intake::Accepted => Ok(TransportResponse {
                status: 202,
                body: Vec::new(),
            }),
            Intake::Unreachable => Err(TransportFailure::Request(TransportError::Closed)),
        }
    }
}

fn decoded_body(request: &PostRequest) -> Json {
    let gzipped = request
        .headers
        .get("content-encoding")
        .is_some_and(|value| value == "gzip");
    let mut decoded = Vec::new();
    if gzipped {
        GzDecoder::new(request.body.as_slice())
            .read_to_end(&mut decoded)
            .expect("the request body is gzip");
    } else {
        decoded.clone_from(&request.body);
    }
    serde_json::from_slice(&decoded).expect("the request body is JSON")
}

fn generated_id() -> String {
    "generated-id".to_owned()
}

fn datadog_init() -> DatadogInit {
    DatadogInit {
        dd_api_key: None,
        dd_site: None,
        dd_agent_host: Some("agent.local".to_owned()),
        dd_agent_port: None,
        allow_env_credentials: true,
        params: DatadogInitParams::default(),
        use_v1: false,
    }
}

pub(crate) fn build_logger(
    environment: &[(&str, &str)],
    intake: Intake,
) -> (Arc<DatadogLogger>, Arc<RecordingTransport>) {
    let transport = RecordingTransport::new(intake);
    let dyn_transport: Arc<dyn Transport> = Arc::<RecordingTransport>::clone(&transport);
    let values = lookup(environment);
    let logger = DatadogLogger::new(
        &datadog_init(),
        &values,
        DatadogDeps {
            transport: dyn_transport,
            clock: Arc::new(FixedClock(Duration::from_secs(1))),
            ids: generated_id,
            tasks: None,
        },
    )
    .expect("the logger initializes");
    (logger, transport)
}

pub(crate) fn last_queued(logger: &DatadogLogger) -> DatadogPayload {
    logger
        .queue
        .pending()
        .into_iter()
        .last()
        .expect("a payload is queued")
}

#[derive(Clone, Default)]
pub(crate) struct CapturedLogs(Arc<Mutex<Vec<(Level, String)>>>);

impl CapturedLogs {
    pub(crate) fn count_containing(&self, level: Level, needle: &str) -> usize {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|(event_level, message)| *event_level == level && message.contains(needle))
            .count()
    }
}

#[derive(Default)]
struct EventMessage(String);

impl Visit for EventMessage {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for CapturedLogs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut message = EventMessage::default();
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((*event.metadata().level(), message.0));
    }
}

pub(crate) fn capture_logs() -> (DefaultGuard, CapturedLogs) {
    let logs = CapturedLogs::default();
    let guard = set_default(tracing_subscriber::registry().with(logs.clone()));
    (guard, logs)
}
