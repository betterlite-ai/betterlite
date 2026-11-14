use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::future::pending;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::azure_sentinel_logging::audit::StandardAuditLogPayload;
use crate::azure_sentinel_logging::logger::{AzureSentinelLogger, SentinelDeps};
use crate::azure_sentinel_logging::settings::{SentinelInit, SentinelSettings};
use crate::batch::logger::{BatchQueue, BatchSettings};
use crate::callback_logger::event::LoggerEvent;
use crate::datadog_logging::clock::Clock;
use crate::datadog_logging::transport::{
    PostRequest, Transport, TransportFailure, TransportResponse,
};
use crate::fields::payload::StandardLoggingPayload;
use crate::fields::status::LoggingStatus;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

pub(super) type Environment = BTreeMap<String, String>;

pub(super) const MODEL_URL: &str = "https://dce.example.test/dataCollectionRules/dcr-1/streams/Custom-LiteLLM?api-version=2023-01-01";
pub(super) const AUDIT_URL: &str = "https://dce.example.test/dataCollectionRules/dcr-1/streams/Custom-Audit?api-version=2023-01-01";
pub(super) const TOKEN_URL: &str = "https://login.microsoftonline.com/tenant-1/oauth2/v2.0/token";

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

pub(super) fn environment(pairs: &[(&str, &str)]) -> Environment {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn required_environment() -> Environment {
    environment(&[
        ("AZURE_SENTINEL_DCR_IMMUTABLE_ID", "dcr-1"),
        ("AZURE_SENTINEL_ENDPOINT", "https://dce.example.test"),
        ("AZURE_SENTINEL_TENANT_ID", "tenant-1"),
        ("AZURE_SENTINEL_CLIENT_ID", "client-1"),
        ("AZURE_SENTINEL_CLIENT_SECRET", "secret-1"),
    ])
}

pub(super) fn init_with_secret(secret: &str) -> SentinelInit {
    SentinelInit {
        dcr_immutable_id: None,
        stream_name: None,
        endpoint: None,
        tenant_id: None,
        client_id: None,
        client_secret: Some(SecretString::from(secret)),
        audit_stream_name: None,
        authority_host: None,
    }
}

pub(super) fn model_payload(id: &str, failure: bool) -> StandardLoggingPayload {
    let mut payload: StandardLoggingPayload =
        serde_json::from_str(STANDARD_PAYLOAD).expect("fixture is a standard logging payload");
    payload.id = id.to_owned();
    if failure {
        payload.status = LoggingStatus::Failure;
        payload.status_fields.llm_api_status = LoggingStatus::Failure;
    }
    payload
}

pub(super) fn padded_payload(id: &str, padding: usize) -> StandardLoggingPayload {
    let mut payload = model_payload(id, false);
    payload
        .model_parameters
        .insert("pad".to_owned(), Json::Text("x".repeat(padding)));
    payload
}

pub(super) fn audit_payload(id: &str) -> StandardAuditLogPayload {
    StandardAuditLogPayload {
        id: id.to_owned(),
        updated_at: "2024-01-01T00:00:00Z".to_owned(),
        changed_by: "admin".to_owned(),
        hashed_changed_by_api_key: "hashed-key".to_owned(),
        action: "created".to_owned(),
        table_name: "LiteLLM_VerificationToken".to_owned(),
        object_id: "object-1".to_owned(),
        before_value: None,
        updated_values: Some("{}".to_owned()),
    }
}

#[derive(Debug)]
pub(super) struct ManualClock(Mutex<Duration>);

impl ManualClock {
    pub(super) fn at(seconds: u64) -> Arc<Self> {
        Arc::new(Self(Mutex::new(Duration::from_secs(seconds))))
    }

    pub(super) fn advance(&self, seconds: u64) {
        let mut now = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        *now = now.saturating_add(Duration::from_secs(seconds));
    }
}

impl Clock for ManualClock {
    fn now_unix(&self) -> Duration {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[derive(Debug)]
pub(super) enum Reply {
    Status { status: u16, body: String },
    Network,
    Pending,
}

impl Reply {
    pub(super) fn token() -> Self {
        Self::body(200, r#"{"access_token": "token-1", "expires_in": 3600}"#)
    }

    pub(super) fn body(status: u16, body: &str) -> Self {
        Self::Status {
            status,
            body: body.to_owned(),
        }
    }

    pub(super) fn status(status: u16) -> Self {
        Self::body(status, "")
    }
}

#[derive(Clone, Debug)]
pub(super) struct Call {
    pub(super) url: String,
    pub(super) headers: Vec<(String, String)>,
    pub(super) body: String,
}

impl Call {
    pub(super) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    pub(super) fn entries(&self) -> Vec<IndexMap<String, Json>> {
        serde_json::from_str(&self.body).unwrap_or_default()
    }

    pub(super) fn ids(&self) -> Vec<String> {
        let identified: Vec<Identified> = serde_json::from_str(&self.body).unwrap_or_default();
        identified.into_iter().map(|entry| entry.id).collect()
    }
}

#[derive(Deserialize)]
struct Identified {
    id: String,
}

type Arrival = Box<dyn FnOnce() + Send>;

#[derive(Default)]
pub(super) struct ScriptedTransport {
    replies: Mutex<VecDeque<Reply>>,
    calls: Mutex<Vec<Call>>,
    arrival: Mutex<Option<Arrival>>,
}

impl ScriptedTransport {
    pub(super) fn new(replies: Vec<Reply>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(VecDeque::from(replies)),
            calls: Mutex::new(Vec::new()),
            arrival: Mutex::new(None),
        })
    }

    pub(super) fn calls(&self) -> Vec<Call> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn urls(&self) -> Vec<String> {
        self.calls().into_iter().map(|call| call.url).collect()
    }

    pub(super) fn batch_ids(&self) -> Vec<Vec<String>> {
        self.calls()
            .iter()
            .filter(|call| call.url != TOKEN_URL)
            .map(Call::ids)
            .collect()
    }

    // runs once, as the next batch request leaves, to model events that arrive while it is in flight
    pub(super) fn on_next_batch(&self, arrival: Arrival) {
        *self.arrival.lock().unwrap_or_else(PoisonError::into_inner) = Some(arrival);
    }

    fn record(&self, request: PostRequest) {
        let headers = request
            .headers
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_owned(),
                    value.to_str().expect("header value is text").to_owned(),
                )
            })
            .collect();
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Call {
                url: request.url,
                headers,
                body: String::from_utf8(request.body).expect("the body is UTF-8"),
            });
    }

    fn next_reply(&self) -> Reply {
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .expect("the scripted transport has a reply for every request")
    }
}

#[async_trait]
impl Transport for ScriptedTransport {
    async fn post(&self, request: PostRequest) -> Result<TransportResponse, TransportFailure> {
        let is_token = request.url == TOKEN_URL;
        self.record(request);
        let arrival = self
            .arrival
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take_if(|_| !is_token);
        if let Some(arrive) = arrival {
            arrive();
        }
        match self.next_reply() {
            Reply::Status { status, body } => Ok(TransportResponse {
                status,
                body: body.into_bytes(),
            }),
            Reply::Network => Err(TransportFailure::Request(TransportError::Rejected(
                HttpError::Timeout,
            ))),
            Reply::Pending => {
                pending::<()>().await;
                Err(TransportFailure::Request(TransportError::Rejected(
                    HttpError::Timeout,
                )))
            }
        }
    }
}

pub(super) struct Rig {
    pub(super) logger: Arc<AzureSentinelLogger>,
    pub(super) transport: Arc<ScriptedTransport>,
    pub(super) clock: Arc<ManualClock>,
    pub(super) stopped: CancellationToken,
}

pub(super) fn rig(replies: Vec<Reply>, batch_size: usize, max_queue_size: usize) -> Rig {
    let batch = BatchSettings {
        batch_size,
        flush_interval: Duration::from_secs(3600),
        max_queue_size,
    };
    timed_rig(replies, batch, None)
}

pub(super) fn timed_rig(
    replies: Vec<Reply>,
    batch: BatchSettings,
    tasks: Option<TaskTracker>,
) -> Rig {
    let mut variables = required_environment();
    variables.insert(
        "AZURE_SENTINEL_AUDIT_STREAM_NAME".to_owned(),
        "Custom-Audit".to_owned(),
    );
    let mut settings = SentinelSettings::resolve(&SentinelInit::default(), &variables)
        .expect("the environment resolves");
    settings.batch = batch;
    let transport = ScriptedTransport::new(replies);
    let clock = ManualClock::at(1_000);
    let concrete_transport = Arc::clone(&transport);
    let dyn_transport: Arc<dyn Transport> = concrete_transport;
    let concrete_clock = Arc::clone(&clock);
    let dyn_clock: Arc<dyn Clock> = concrete_clock;
    let stopped = CancellationToken::new();
    let logger = AzureSentinelLogger::new(
        settings,
        SentinelDeps {
            transport: dyn_transport,
            clock: dyn_clock,
            stopped: stopped.clone(),
            tasks,
        },
    );
    Rig {
        logger,
        transport,
        clock,
        stopped,
    }
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

pub(super) async fn log_models(rig: &Rig, ids: &[&str]) {
    for id in ids {
        rig.logger
            .async_log_model_event(Some(&model_payload(id, false)))
            .await;
    }
}

pub(super) async fn log_audits(rig: &Rig, ids: &[&str]) {
    for id in ids {
        rig.logger
            .async_log_audit_log_event(audit_payload(id))
            .await;
    }
}

pub(super) fn arrive_during_the_next_send<E>(
    rig: &Rig,
    queue: &BatchQueue<E>,
    ids: &[&str],
    record: fn(&str) -> E,
) where
    E: Send + 'static,
{
    let arrivals = queue.clone();
    let arriving: Vec<String> = ids.iter().map(|id| (*id).to_owned()).collect();
    rig.transport.on_next_batch(Box::new(move || {
        for id in &arriving {
            let _depth = arrivals.enqueue(record(id));
        }
    }));
}

pub(super) trait HasId {
    fn id(&self) -> &str;
}

impl HasId for StandardLoggingPayload {
    fn id(&self) -> &str {
        &self.id
    }
}

impl HasId for StandardAuditLogPayload {
    fn id(&self) -> &str {
        &self.id
    }
}

pub(super) fn pending_ids<E>(queue: &BatchQueue<E>) -> Vec<String>
where
    E: Clone + HasId,
{
    queue
        .pending()
        .iter()
        .map(|record| record.id().to_owned())
        .collect()
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
        if *event.metadata().level() > tracing::Level::WARN {
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

pub(super) fn capture_logs() -> (DefaultGuard, CapturedLogs) {
    let logs = CapturedLogs::default();
    let guard = set_default(tracing_subscriber::registry().with(logs.clone()));
    (guard, logs)
}
