use std::collections::{BTreeMap, VecDeque};
use std::convert::Infallible;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use tokio_util::task::TaskTracker;

use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;

use crate::callback_logger::event::LoggerEvent;
use crate::datadog::apm::{Span, Tracer};
use crate::datadog_llm_metrics::token_counting::ToolTokenCounter;
use crate::datadog_llm_obs::span_records::LLMObsPayload;
use crate::datadog_llm_observability::error::DatadogLlmObsError;
use crate::datadog_llm_observability::logger::{DatadogLlmObsDeps, DatadogLlmObsLogger};
use crate::datadog_llm_observability::settings::{
    DatadogLlmObsInit, DatadogLlmObsInitParams, DatadogLlmObsSettings,
};
use crate::datadog_logging::transport::{
    PostRequest, Transport, TransportFailure, TransportResponse,
};
use crate::fields::payload::StandardLoggingPayload;
use crate::fields::status::LoggingStatus;

pub(super) type Environment = BTreeMap<String, String>;

pub(super) const STANDARD_PAYLOAD: &str = r#"{
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
    "response_time": 1.5,
    "model_map_information": {},
    "model": "gpt-4o",
    "api_base": "https://api.openai.com",
    "metadata": {},
    "saved_cache_cost": 0.0,
    "request_tags": [],
    "model_parameters": {},
    "hidden_params": {},
    "messages": [{"role": "user", "content": "hi"}],
    "response": {"choices": [{"message": {"role": "assistant", "content": "hello"}}]}
}"#;

pub(super) fn standard_payload(id: &str) -> StandardLoggingPayload {
    let mut payload: StandardLoggingPayload =
        serde_json::from_str(STANDARD_PAYLOAD).expect("the fixture is a standard logging payload");
    payload.id = id.to_owned();
    payload
}

pub(super) fn failure_payload(id: &str) -> StandardLoggingPayload {
    let mut payload = standard_payload(id);
    payload.status = LoggingStatus::Failure;
    payload.status_fields.llm_api_status = LoggingStatus::Failure;
    payload
}

pub(super) fn event(payload: StandardLoggingPayload) -> Arc<LoggerEvent> {
    event_with(payload, Json::Object(IndexMap::new()))
}

pub(super) fn event_with(
    payload: StandardLoggingPayload,
    dynamic_params: Json,
) -> Arc<LoggerEvent> {
    Arc::new(LoggerEvent {
        payload: Arc::new(payload),
        dynamic_params: Arc::new(dynamic_params),
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

pub(super) fn init(
    key: Option<&str>,
    site: Option<&str>,
    agent_host: Option<&str>,
    agent_port: Option<&str>,
) -> DatadogLlmObsInit {
    DatadogLlmObsInit {
        dd_api_key: key.map(secrecy::SecretString::from),
        dd_site: site.map(str::to_owned),
        dd_agent_host: agent_host.map(str::to_owned),
        dd_agent_port: agent_port.map(str::to_owned),
        turn_off_message_logging: None,
        message_logging: None,
        batch_size: None,
        flush_interval_seconds: None,
        params: None,
        global_turn_off: false,
    }
}

pub(super) fn agent_init() -> DatadogLlmObsInit {
    init(None, None, Some("agent.local"), None)
}

pub(super) fn params(turn_off_message_logging: Option<bool>) -> DatadogLlmObsInitParams {
    DatadogLlmObsInitParams {
        turn_off_message_logging,
    }
}

pub(super) fn resolve(
    init: &DatadogLlmObsInit,
    environment: &Environment,
) -> Result<DatadogLlmObsSettings, DatadogLlmObsError> {
    DatadogLlmObsSettings::resolve(init, environment)
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Outcome {
    Reply { status: u16, body: &'static str },
    Network,
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
            .find(|(header_name, _value)| header_name.eq_ignore_ascii_case(name))
            .map(|(_name, value)| value.as_str())
    }

    pub(super) fn spans(&self) -> Vec<Json> {
        match field(&self.body, &["data", "attributes", "spans"]) {
            Some(Json::List(spans)) => spans.clone(),
            Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_),
            )
            | None => Vec::new(),
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
            body: serde_json::from_slice(&request.body).expect("the body is JSON"),
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
        }
    }
}

pub(super) struct TestSpan {
    pub(super) trace_id: Option<u128>,
}

impl Span for TestSpan {
    fn trace_id(&self) -> Option<u128> {
        self.trace_id
    }
}

impl TestSpan {
    fn borrowed(&self) -> &dyn Span {
        self
    }
}

pub(super) struct TestTracer {
    pub(super) span: Option<TestSpan>,
}

impl Tracer for TestTracer {
    fn current_span(&self) -> Option<&dyn Span> {
        self.span.as_ref().map(TestSpan::borrowed)
    }
}

pub(super) struct BareTracer;

impl Tracer for BareTracer {}

#[derive(Default)]
pub(super) struct TestCounter {
    pub(super) calls: Mutex<Vec<(String, String)>>,
}

impl ToolTokenCounter for TestCounter {
    type Failure = Infallible;

    fn count_text(
        &self,
        model: &str,
        text: &str,
    ) -> impl Future<Output = Result<usize, Self::Failure>> + Send {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((model.to_owned(), text.to_owned()));
        ready(Ok(text.chars().count()))
    }
}

pub(super) fn field<'value>(value: &'value Json, path: &[&str]) -> Option<&'value Json> {
    let mut current = value;
    for key in path {
        let Json::Object(fields) = current else {
            return None;
        };
        current = fields.get(*key)?;
    }
    Some(current)
}

pub(super) fn empty_span() -> LLMObsPayload {
    LLMObsPayload {
        parent_id: None,
        trace_id: None,
        apm_id: None,
        span_id: None,
        name: None,
        meta: None,
        start_ns: None,
        duration: None,
        metrics: None,
        tags: None,
        status: None,
    }
}

pub(super) fn generated_id() -> String {
    "generated-id".to_owned()
}

pub(super) type Logger = DatadogLlmObsLogger<TestCounter>;

pub(super) fn null_tracer() -> Arc<dyn Tracer + Send + Sync> {
    Arc::new(BareTracer)
}

pub(super) fn build(
    init: &DatadogLlmObsInit,
    environment: &Environment,
    replies: Vec<Reply>,
) -> (
    Arc<DatadogLlmObsLogger<TestCounter>>,
    Arc<ScriptedTransport>,
) {
    build_with(init, environment, replies, null_tracer(), None)
}

pub(super) fn build_with(
    init: &DatadogLlmObsInit,
    environment: &Environment,
    replies: Vec<Reply>,
    tracer: Arc<dyn Tracer + Send + Sync>,
    tasks: Option<TaskTracker>,
) -> (
    Arc<DatadogLlmObsLogger<TestCounter>>,
    Arc<ScriptedTransport>,
) {
    let transport = ScriptedTransport::new(replies);
    let concrete: Arc<ScriptedTransport> = Arc::clone(&transport);
    let logger = DatadogLlmObsLogger::new(
        init,
        environment,
        Arc::new(TestCounter::default()),
        DatadogLlmObsDeps {
            transport: concrete,
            tracer,
            ids: generated_id,
            tasks,
        },
    )
    .expect("the logger initializes");
    (logger, transport)
}
