use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::io::{Result as IoResult, Write};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use jiff::tz::TimeZone;
use serde_json::Number;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::Level;
use tracing::subscriber::with_default;
use tracing_subscriber::fmt::MakeWriter;

use crate::datadog::cost_management::datadog_cost_management::logger::{
    CostManagementDeps, DatadogCostManagementLogger,
};
use crate::datadog::cost_management::datadog_cost_management::settings::{
    CostManagementInit, CostManagementSettings,
};
use crate::datadog::cost_management::datadog_cost_management::transport::{
    PutRequest, PutResponse, Transport,
};
use crate::fields::payload::StandardLoggingPayload;

pub(super) const BASE_PAYLOAD: &str = r#"{
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
    "startTime": 1704110400.0,
    "endTime": 1704110401.0,
    "completionStartTime": 1704110400.5,
    "response_time": 0.5,
    "model_map_information": {},
    "model": "gpt-4o",
    "model_group": "production",
    "custom_llm_provider": "openai",
    "model_id": "router-1",
    "api_base": "https://api.openai.com",
    "metadata": {},
    "saved_cache_cost": 0.0,
    "request_tags": [],
    "model_parameters": {},
    "hidden_params": {}
}"#;

#[derive(Clone, Default)]
struct Buffer(Arc<Mutex<Vec<u8>>>);

impl Buffer {
    fn contents(&self) -> String {
        String::from_utf8(
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        )
        .expect("the captured log output is UTF-8")
    }
}

impl Write for Buffer {
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

impl<'writer> MakeWriter<'writer> for Buffer {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

pub(super) fn logged<T, F>(action: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    let buffer = Buffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    let result = with_default(subscriber, action);
    (result, buffer.contents())
}

#[derive(Debug, thiserror::Error)]
#[error("the scripted transport has no reply left")]
pub(super) struct NoReply;

#[derive(Clone, Debug)]
pub(super) struct RecordedRequest {
    pub(super) url: String,
    pub(super) headers: BTreeMap<String, String>,
    pub(super) body: Json,
}

pub(super) struct ScriptedTransport {
    replies: Mutex<VecDeque<u16>>,
    requests: Mutex<Vec<RecordedRequest>>,
}

impl ScriptedTransport {
    pub(super) fn new(statuses: &[u16]) -> Self {
        Self {
            replies: Mutex::new(statuses.iter().copied().collect()),
            requests: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn requests(&self) -> Vec<RecordedRequest> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Transport for ScriptedTransport {
    type Error = NoReply;

    fn put(
        &self,
        request: PutRequest,
    ) -> impl Future<Output = Result<PutResponse, Self::Error>> + Send {
        let headers = request
            .headers
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_owned(),
                    value.to_str().unwrap_or_default().to_owned(),
                )
            })
            .collect();
        let body = serde_json::from_slice::<Json>(&request.body).expect("a JSON cost entry body");
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedRequest {
                url: request.url.to_string(),
                headers,
                body,
            });
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        async move { reply.map_or(Err(NoReply), |status| Ok(PutResponse { status })) }
    }
}

pub(super) fn number(value: f64) -> Json {
    Json::Number(Number::from_f64(value).expect("a finite test number"))
}

pub(super) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(super) fn list(values: Vec<Json>) -> Json {
    Json::List(values)
}

pub(super) fn object(entries: Vec<(&str, Json)>) -> Json {
    Json::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

pub(super) fn object_field<'json>(value: &'json Json, key: &str) -> Option<&'json Json> {
    match value {
        Json::Object(map) => map.get(key),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

pub(super) fn text_value(value: &Json) -> Option<&str> {
    match value {
        Json::Text(text) => Some(text),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

pub(super) fn payload(value: &Json) -> Arc<StandardLoggingPayload> {
    let text = serde_json::to_string(value).expect("the test payload encodes");
    Arc::new(serde_json::from_str(&text).expect("a valid standard logging payload"))
}

pub(super) fn default_payload() -> Raw {
    Raw(serde_json::from_str(BASE_PAYLOAD).expect("the base payload is valid JSON"))
}

pub(super) struct Raw(pub(super) Json);

impl Raw {
    pub(super) fn set(mut self, path: &str, value: Json) -> Self {
        let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
        set_at(&mut self.0, &parts, value);
        self
    }

    pub(super) fn build(self) -> Arc<StandardLoggingPayload> {
        payload(&self.0)
    }
}

fn set_at(target: &mut Json, parts: &[&str], value: Json) {
    let Some((head, rest)) = parts.split_first() else {
        *target = value;
        return;
    };
    if !matches!(target, Json::Object(_)) {
        *target = Json::Object(IndexMap::new());
    }
    if let Json::Object(map) = target {
        let child = map.entry((*head).to_owned()).or_insert(Json::Null);
        set_at(child, rest, value);
    }
}

#[must_use]
pub(super) fn environment() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("DD_API_KEY".to_owned(), "api-key".to_owned()),
        ("DD_APP_KEY".to_owned(), "app-key".to_owned()),
        ("DD_SITE".to_owned(), "test.datadoghq.com".to_owned()),
        ("DD_ENV".to_owned(), "prod".to_owned()),
        ("DD_SERVICE".to_owned(), "gateway".to_owned()),
        ("HOSTNAME".to_owned(), "host-1".to_owned()),
        ("POD_NAME".to_owned(), "pod-1".to_owned()),
    ])
}

#[must_use]
pub(super) fn init(cost_tag_keys: &[&str]) -> CostManagementInit {
    CostManagementInit {
        cost_tag_keys: cost_tag_keys.iter().map(|key| (*key).to_owned()).collect(),
        batch_size: Some(1_000),
        flush_interval: None,
        max_queue_size: None,
    }
}

#[must_use]
pub(super) fn settings(init: &CostManagementInit) -> CostManagementSettings {
    settings_with(init, &environment())
}

#[must_use]
pub(super) fn settings_with(
    init: &CostManagementInit,
    environment: &BTreeMap<String, String>,
) -> CostManagementSettings {
    CostManagementSettings::resolve(init, environment).expect("the settings resolve")
}

#[must_use]
pub(super) fn tag<'tags>(tags: &'tags IndexMap<String, String>, key: &str) -> Option<&'tags str> {
    tags.get(key).map(String::as_str)
}

pub(super) struct Fixture {
    pub(super) init: CostManagementInit,
    pub(super) environment: BTreeMap<String, String>,
    pub(super) transport: Arc<ScriptedTransport>,
    pub(super) zone: TimeZone,
    pub(super) tasks: Option<TaskTracker>,
    pub(super) shutdown: CancellationToken,
}

impl Fixture {
    pub(super) fn new(statuses: &[u16]) -> Self {
        Self {
            init: init(&[]),
            environment: environment(),
            transport: Arc::new(ScriptedTransport::new(statuses)),
            zone: TimeZone::UTC,
            tasks: None,
            shutdown: CancellationToken::new(),
        }
    }

    #[must_use]
    pub(super) fn with_init(mut self, init: CostManagementInit) -> Self {
        self.init = init;
        self
    }

    #[must_use]
    pub(super) fn with_tasks(mut self) -> Self {
        self.tasks = Some(TaskTracker::new());
        self
    }

    pub(super) fn build(&self) -> Arc<DatadogCostManagementLogger<ScriptedTransport>> {
        DatadogCostManagementLogger::new(
            &self.init,
            &self.environment,
            CostManagementDeps {
                transport: Arc::clone(&self.transport),
                zone: self.zone.clone(),
                tasks: self.tasks.clone(),
                shutdown: self.shutdown.clone(),
            },
        )
        .expect("the cost logger builds")
    }
}
