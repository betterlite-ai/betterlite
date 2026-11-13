use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::io::{Result as IoResult, Write};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_models::decisions::Json;
use chrono::{DateTime, TimeZone as _, Utc};
use indexmap::IndexMap;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use crate::argilla_logging::logger::RandomSource;
use crate::callback_logger::event::LoggerEvent;
use crate::fields::payload::StandardLoggingPayload;
use crate::http_analytics_loggers::delivery::{Delivery, PostRequest, PostResponse};
use crate::langsmith_logger::clock::Clock;
use crate::langsmith_logger::settings::{LangsmithSettings, resolve};
use crate::langsmith_logger::{LangsmithDeps, LangsmithLogger};
use crate::langsmith_payloads::ids::IdSource;

pub(crate) const PAYLOAD: &str = r#"{
    "id": "chatcmpl-1",
    "trace_id": "trace-1",
    "session_id": "session-1",
    "call_type": "completion",
    "response_cost": 0.5,
    "status": "success",
    "status_fields": {"llm_api_status": "success", "guardrail_status": "not_run"},
    "total_tokens": 30,
    "prompt_tokens": 10,
    "completion_tokens": 20,
    "startTime": 1704067200.0,
    "endTime": 1704067201.5,
    "completionStartTime": 1704067200.5,
    "response_time": 1.0,
    "model_map_information": {"model_map_key": "", "model_map_value": null},
    "model": "gpt-4o",
    "api_base": "https://api.openai.com",
    "metadata": {"requester_metadata": {"user_api_key": "secret", "session_id": "nested-session"}},
    "saved_cache_cost": 0.0,
    "request_tags": ["team-a"],
    "model_parameters": {},
    "hidden_params": {},
    "response": {"id": "chatcmpl-1"},
    "error_str": null
}"#;

pub(crate) fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(crate) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(crate) fn dynamic(pairs: &[(&str, Json)]) -> Json {
    let mut fields = IndexMap::new();
    for (name, value) in pairs {
        fields.insert((*name).to_owned(), value.clone());
    }
    Json::Object(fields)
}

pub(crate) fn payload() -> StandardLoggingPayload {
    serde_json::from_str(PAYLOAD).expect("the sample payload parses")
}

pub(crate) fn event(parameters: Json) -> LoggerEvent {
    LoggerEvent {
        payload: Arc::new(payload()),
        dynamic_params: Arc::new(parameters),
        litellm_metadata: Arc::new(IndexMap::new()),
        stream: false,
        guardrail_snapshot: None,
    }
}

pub(crate) struct FixedIds(pub &'static str);

impl IdSource for FixedIds {
    fn next_id(&self) -> String {
        self.0.to_owned()
    }
}

pub(crate) struct FixedClock(pub DateTime<Utc>);

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

pub(crate) fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2024, 1, 2, 3, 4, 5)
        .single()
        .expect("a representable instant")
}

pub(crate) struct ScriptedRandom {
    draws: Mutex<VecDeque<f64>>,
}

impl ScriptedRandom {
    pub(crate) fn new(draws: &[f64]) -> Self {
        Self {
            draws: Mutex::new(draws.iter().copied().collect()),
        }
    }
}

impl RandomSource for ScriptedRandom {
    fn sample(&self) -> f64 {
        self.draws
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Recorded {
    pub(crate) url: String,
    pub(crate) headers: http::HeaderMap,
    pub(crate) body: Vec<u8>,
}

#[derive(serde::Deserialize)]
struct BatchBody {
    post: Vec<Json>,
}

impl Recorded {
    pub(crate) fn runs(&self) -> Vec<Json> {
        serde_json::from_slice::<BatchBody>(&self.body)
            .expect("the batch body carries a post list")
            .post
    }
}

pub(crate) fn object_field<'value>(value: &'value Json, name: &str) -> Option<&'value Json> {
    match value {
        Json::Object(fields) => fields.get(name),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

pub(crate) fn text_field<'value>(value: &'value Json, name: &str) -> Option<&'value str> {
    match object_field(value, name) {
        Some(Json::Text(text)) => Some(text),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the fake LangSmith endpoint cannot be reached")]
pub(crate) struct FakeError;

#[derive(Clone)]
pub(crate) struct FakeDelivery {
    posts: Arc<Mutex<Vec<Recorded>>>,
    status: Arc<Mutex<u16>>,
    failing: Arc<Mutex<bool>>,
    notify: Option<mpsc::Sender<Recorded>>,
}

impl FakeDelivery {
    pub(crate) fn answering(status: u16) -> Self {
        Self {
            posts: Arc::new(Mutex::new(Vec::new())),
            status: Arc::new(Mutex::new(status)),
            failing: Arc::new(Mutex::new(false)),
            notify: None,
        }
    }

    pub(crate) fn failing() -> Self {
        let delivery = Self::answering(200);
        *delivery
            .failing
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = true;
        delivery
    }

    pub(crate) fn subscribed(&self) -> (Self, mpsc::Receiver<Recorded>) {
        let (sender, receiver) = mpsc::channel(64);
        let mut subscribed = self.clone();
        subscribed.notify = Some(sender);
        (subscribed, receiver)
    }

    pub(crate) fn posts(&self) -> Vec<Recorded> {
        self.posts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Delivery for FakeDelivery {
    type Error = FakeError;

    fn post(
        &self,
        request: PostRequest,
    ) -> impl Future<Output = Result<PostResponse, Self::Error>> + Send {
        let recorded = Recorded {
            url: request.url.to_string(),
            headers: request.headers,
            body: request.body,
        };
        let posts = Arc::clone(&self.posts);
        let status = *self.status.lock().unwrap_or_else(PoisonError::into_inner);
        let failing = *self.failing.lock().unwrap_or_else(PoisonError::into_inner);
        let notify = self.notify.clone();
        async move {
            posts
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(recorded.clone());
            if let Some(sender) = notify {
                let _subscriber_dropped = sender.send(recorded).await;
            }
            if failing {
                return Err(FakeError);
            }
            Ok(PostResponse {
                status,
                body: b"the endpoint answered".to_vec(),
            })
        }
    }
}

pub(crate) struct Options<'values> {
    pub(crate) environment: &'values [(&'values str, &'values str)],
    pub(crate) batch_size: Option<i64>,
    pub(crate) draws: &'values [f64],
    pub(crate) redact: bool,
}

pub(crate) struct Harness {
    pub(crate) logger: Arc<LangsmithLogger<FakeDelivery, ScriptedRandom>>,
    pub(crate) delivery: FakeDelivery,
    pub(crate) tasks: TaskTracker,
    pub(crate) shutdown: CancellationToken,
}

pub(crate) fn settings(options: &Options<'_>) -> LangsmithSettings {
    resolve(
        &environment(options.environment),
        options.batch_size,
        options.redact,
    )
    .expect("the LangSmith settings resolve")
}

pub(crate) fn harness(options: &Options<'_>, delivery: FakeDelivery) -> Harness {
    let tasks = TaskTracker::new();
    let shutdown = CancellationToken::new();
    let logger = LangsmithLogger::new(
        settings(options),
        LangsmithDeps {
            delivery: delivery.clone(),
            environment: Arc::new(environment(options.environment)),
            random: ScriptedRandom::new(options.draws),
            ids: Arc::new(FixedIds("generated-id")),
            clock: Arc::new(FixedClock(now())),
            tasks: tasks.clone(),
            shutdown: shutdown.clone(),
        },
    );
    Harness {
        logger,
        delivery,
        tasks,
        shutdown,
    }
}

#[derive(Clone, Default)]
pub(crate) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub(crate) fn contents(&self) -> String {
        String::from_utf8(
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        )
        .expect("tracing writes UTF-8")
    }
}

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for Captured {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

pub(crate) fn capture_logs() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(move || writer.clone())
        .finish();
    (captured, set_default(subscriber))
}
