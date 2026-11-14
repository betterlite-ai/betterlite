use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::future::pending;
use std::io::Read as _;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use flate2::read::GzDecoder;
use indexmap::IndexMap;
use secrecy::SecretString;
use tokio_util::task::TaskTracker;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default, with_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::callback_logger::event::LoggerEvent;
use crate::datadog::payloads::status::DataDogStatus;
use crate::datadog::payloads::types::DatadogPayload;
use crate::datadog_logging::clock::Clock;
use crate::datadog_logging::logger::{DatadogDeps, DatadogLogger};
use crate::datadog_logging::settings::{DatadogInit, DatadogInitParams};
use crate::datadog_logging::transport::{
    PostRequest, Transport, TransportFailure, TransportResponse,
};
use crate::fields::payload::StandardLoggingPayload;
use crate::fields::status::LoggingStatus;

pub(super) type Environment = BTreeMap<String, String>;

const STANDARD_PAYLOAD: &str = r#"{
    "id": "fixture-id",
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
    "model_map_information": {},
    "model": "gpt-4o",
    "api_base": "https://api.openai.com",
    "metadata": {},
    "saved_cache_cost": 0.0,
    "request_tags": [],
    "model_parameters": {},
    "hidden_params": {}
}"#;

pub(super) fn standard_payload(id: &str, failure: bool) -> StandardLoggingPayload {
    let mut payload: StandardLoggingPayload =
        serde_json::from_str(STANDARD_PAYLOAD).expect("fixture is a standard logging payload");
    payload.id = id.to_owned();
    if failure {
        payload.status = LoggingStatus::Failure;
        payload.status_fields.llm_api_status = LoggingStatus::Failure;
    }
    payload
}

pub(super) fn event(payload: StandardLoggingPayload) -> Arc<LoggerEvent> {
    Arc::new(LoggerEvent {
        payload: Arc::new(payload),
        dynamic_params: Arc::new(Json::Object(IndexMap::new())),
        litellm_metadata: Arc::new(IndexMap::new()),
        stream: false,
        guardrail_snapshot: None,
    })
}

pub(super) fn environment(pairs: &[(&str, &str)]) -> Environment {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn init_with(
    key: Option<&str>,
    site: Option<&str>,
    host: Option<&str>,
    port: Option<&str>,
    allow_env_credentials: bool,
) -> DatadogInit {
    DatadogInit {
        dd_api_key: key.map(SecretString::from),
        dd_site: site.map(str::to_owned),
        dd_agent_host: host.map(str::to_owned),
        dd_agent_port: port.map(str::to_owned),
        allow_env_credentials,
        params: DatadogInitParams::default(),
        use_v1: false,
    }
}

pub(super) fn agent_init() -> DatadogInit {
    init_with(None, None, Some("agent.local"), None, true)
}

pub(super) fn direct_init(api_key: &str, site: &str) -> DatadogInit {
    init_with(Some(api_key), Some(site), None, None, true)
}

#[derive(Clone, Copy, Debug)]
pub(super) struct FixedClock(pub(super) Duration);

impl Clock for FixedClock {
    fn now_unix(&self) -> Duration {
        self.0
    }
}

#[derive(Debug, Default)]
pub(super) struct SequenceClock {
    values: Mutex<VecDeque<Duration>>,
}

impl SequenceClock {
    pub(super) fn new(values: &[Duration]) -> Self {
        Self {
            values: Mutex::new(values.iter().copied().collect()),
        }
    }
}

impl Clock for SequenceClock {
    fn now_unix(&self) -> Duration {
        let mut values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        values.pop_front().unwrap_or_default()
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Outcome {
    Reply { status: u16, body: &'static str },
    Network,
    Pending,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Reply(pub(super) Outcome);

impl Reply {
    pub(super) const fn accepted() -> Self {
        Self(Outcome::Reply {
            status: 202,
            body: "",
        })
    }

    pub(super) const fn status(status: u16) -> Self {
        Self(Outcome::Reply { status, body: "" })
    }

    pub(super) const fn body(status: u16, body: &'static str) -> Self {
        Self(Outcome::Reply { status, body })
    }

    pub(super) const fn network() -> Self {
        Self(Outcome::Network)
    }

    pub(super) const fn pending() -> Self {
        Self(Outcome::Pending)
    }
}

#[derive(Clone, Debug)]
pub(super) struct Call {
    pub(super) url: String,
    pub(super) headers: Vec<(String, String)>,
    pub(super) body: Json,
}

impl Call {
    pub(super) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    pub(super) fn entries(&self) -> Vec<Json> {
        match &self.body {
            Json::List(entries) => entries.clone(),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
                Vec::new()
            }
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct ScriptedTransport {
    replies: Mutex<VecDeque<Reply>>,
    calls: Mutex<Vec<Call>>,
}

impl ScriptedTransport {
    pub(super) fn new(replies: Vec<Reply>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(VecDeque::from(replies)),
            calls: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn calls(&self) -> Vec<Call> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn call_count(&self) -> usize {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

fn body_of(request: &PostRequest) -> Json {
    let gzipped = request
        .headers
        .get("content-encoding")
        .is_some_and(|value| value == "gzip");
    let mut decoded = Vec::new();
    if gzipped {
        GzDecoder::new(request.body.as_slice())
            .read_to_end(&mut decoded)
            .expect("the gzip body decodes");
    } else {
        decoded.clone_from(&request.body);
    }
    serde_json::from_slice(&decoded).expect("the body is JSON")
}

fn record(transport: &ScriptedTransport, request: &PostRequest) {
    let headers = request
        .headers
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_ascii_lowercase(),
                value.to_str().expect("header value is text").to_owned(),
            )
        })
        .collect();
    transport
        .calls
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(Call {
            url: request.url.clone(),
            headers,
            body: body_of(request),
        });
}

#[async_trait]
impl Transport for ScriptedTransport {
    async fn post(&self, request: PostRequest) -> Result<TransportResponse, TransportFailure> {
        record(self, &request);
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .expect("the scripted transport has a reply for every request");
        match reply.0 {
            Outcome::Reply { status, body } => Ok(TransportResponse {
                status,
                body: body.as_bytes().to_vec(),
            }),
            Outcome::Network => Err(TransportFailure::Request(TransportError::Rejected(
                HttpError::Timeout,
            ))),
            Outcome::Pending => {
                pending::<()>().await;
                Err(TransportFailure::Request(TransportError::Rejected(
                    HttpError::Timeout,
                )))
            }
        }
    }
}

pub(super) fn build(
    init: &DatadogInit,
    environment: &Environment,
    replies: Vec<Reply>,
) -> (Arc<DatadogLogger>, Arc<ScriptedTransport>) {
    build_with(
        init,
        environment,
        replies,
        Arc::new(FixedClock(Duration::from_secs(1))),
        None,
    )
}

pub(super) fn build_with(
    init: &DatadogInit,
    environment: &Environment,
    replies: Vec<Reply>,
    clock: Arc<dyn Clock>,
    tasks: Option<TaskTracker>,
) -> (Arc<DatadogLogger>, Arc<ScriptedTransport>) {
    let transport = ScriptedTransport::new(replies);
    let concrete_transport = Arc::clone(&transport);
    let dyn_transport: Arc<dyn Transport> = concrete_transport;
    let logger = DatadogLogger::new(
        init,
        environment,
        DatadogDeps {
            transport: dyn_transport,
            clock,
            ids: generated_id,
            tasks,
        },
    )
    .expect("the logger initializes");
    (logger, transport)
}

pub(super) fn generated_id() -> String {
    "generated-id".to_owned()
}

pub(super) fn last_flush_time(logger: &DatadogLogger) -> Duration {
    *logger
        .last_flush_time
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

pub(super) fn queued(logger: &DatadogLogger) -> Vec<Json> {
    logger.queue.pending().iter().map(queued_payload).collect()
}

fn queued_payload(payload: &DatadogPayload) -> Json {
    let mut fields = IndexMap::new();
    fields.insert("ddsource".to_owned(), Json::Text(payload.ddsource.clone()));
    fields.insert("ddtags".to_owned(), Json::Text(payload.ddtags.clone()));
    fields.insert("hostname".to_owned(), Json::Text(payload.hostname.clone()));
    fields.insert("message".to_owned(), Json::Text(payload.message.clone()));
    fields.insert("service".to_owned(), Json::Text(payload.service.clone()));
    let status = match payload.status {
        DataDogStatus::Info => "info",
        DataDogStatus::Warn => "warning",
        DataDogStatus::Error => "error",
    };
    fields.insert("status".to_owned(), Json::Text(status.to_owned()));
    if let Some(trace_id) = &payload.trace_id {
        fields.insert("dd.trace_id".to_owned(), Json::Text(trace_id.clone()));
    }
    if let Some(span_id) = &payload.span_id {
        fields.insert("dd.span_id".to_owned(), Json::Text(span_id.clone()));
    }
    Json::Object(fields)
}

pub(super) fn message_ids(logger: &DatadogLogger) -> Vec<String> {
    queued(logger).iter().filter_map(message_id).collect()
}

pub(super) fn message_id(payload: &Json) -> Option<String> {
    let Json::Object(fields) = payload else {
        return None;
    };
    let Json::Text(message) = fields.get("message")? else {
        return None;
    };
    let Json::Object(parsed) = serde_json::from_str::<Json>(message).ok()? else {
        return None;
    };
    match parsed.get("id") {
        Some(Json::Text(id)) => Some(id.clone()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

pub(super) fn oversized_payload(message_len: usize) -> DatadogPayload {
    DatadogPayload {
        ddsource: "litellm".to_owned(),
        ddtags: String::new(),
        hostname: String::new(),
        message: "a".repeat(message_len),
        service: "litellm-server".to_owned(),
        status: DataDogStatus::Info,
        trace_id: None,
        span_id: None,
    }
}

pub(super) fn simple_payload() -> DatadogPayload {
    oversized_payload(0)
}

#[derive(Clone, Default)]
pub(super) struct CapturedLogs(Arc<Mutex<Vec<String>>>);

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl CapturedLogs {
    pub(super) fn count_containing(&self, needle: &str) -> usize {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|message| message.contains(needle))
            .count()
    }
}

impl<Inner: Subscriber> Layer<Inner> for CapturedLogs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() > Level::WARN {
            return;
        }
        let mut message = Message::default();
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message.0);
    }
}

pub(super) fn capture<Action, Output>(action: Action) -> (Output, CapturedLogs)
where
    Action: FnOnce() -> Output,
{
    let warnings = CapturedLogs::default();
    let subscriber = tracing_subscriber::registry().with(warnings.clone());
    let output = with_default(subscriber, action);
    (output, warnings)
}

pub(super) fn capture_logs() -> (DefaultGuard, CapturedLogs) {
    let logs = CapturedLogs::default();
    let guard = set_default(tracing_subscriber::registry().with(logs.clone()));
    (guard, logs)
}
