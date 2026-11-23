use std::collections::VecDeque;
use std::io::{Result as IoResult, Write};
use std::str::FromStr as _;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::sections::request_headers::Settings;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::model_registration::JsonObject;
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use http::header::CONTENT_TYPE;
use http::{HeaderMap, HeaderValue};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde_json::Number;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt;
use tracing_subscriber::fmt::MakeWriter;

use crate::fields::payload::StandardLoggingPayload;
use crate::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use crate::logging_state::types::{CallbackPolicy, DynamicCallbacks, RequestLoggingFlags};
use crate::logging_state::{LoggingInit, LoggingState};
use crate::logging_success::ports::StandardPayloadBuilder;
use crate::passthrough_handler::base::BasePassthroughLoggingHandler;
use crate::passthrough_handler::ports::{BaseConfig, PassthroughLoggingProvider, ResponseCost};
use crate::passthrough_handler::types::{
    PassthroughChatInput, PassthroughHandlerError, PassthroughLoggingPayload, ResponsePayloadInput,
    StreamedResponse, UpstreamChatResponse,
};
use crate::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use crate::passthrough_stream_logging::object::{
    StreamLoggingObject, StreamLoggingState, StreamSuccessCall, StreamSuccessDispatch,
};

pub(super) const PAYLOAD: &str = r#"{
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
    "hidden_params": {},
    "messages": [{"role": "user", "content": "hi"}],
    "response": {"id": "chatcmpl-1", "object": "chat.completion", "choices": []}
}"#;

pub(super) fn payload() -> StandardLoggingPayload {
    serde_json::from_str(PAYLOAD).unwrap()
}

pub(super) fn payload_json() -> Json {
    json(&serde_json::to_string(&payload()).unwrap())
}

pub(super) fn time(text: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(text).unwrap()
}

pub(super) fn start() -> DateTime<FixedOffset> {
    time("2024-01-01T00:00:00+00:00")
}

pub(super) fn end() -> DateTime<FixedOffset> {
    time("2024-01-01T00:00:02+00:00")
}

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

pub(super) fn json_object(text: &str) -> JsonObject {
    match json(text) {
        Json::Object(fields) => fields,
        other => panic!("the fixture must be a JSON object, got {other:?}"),
    }
}

pub(super) fn cost(text: &str) -> Decimal {
    Decimal::from_str(text).unwrap()
}

pub(super) fn cost_json(amount: Decimal) -> Json {
    Json::Number(Number::from_str(&amount.to_string()).unwrap())
}

pub(super) fn headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers
}

pub(super) fn upstream(headers: &HeaderMap) -> UpstreamChatResponse<'_> {
    UpstreamChatResponse {
        status_code: 200,
        headers,
        body: b"{}",
    }
}

pub(super) fn kwargs(pairs: &[(&str, Json)]) -> PassThroughLoggingKwargs {
    let entries: IndexMap<String, Json> = pairs
        .iter()
        .map(|(key, value)| (String::from(*key), value.clone()))
        .collect();
    PassThroughLoggingKwargs { entries }
}

pub(super) fn logging_payload(request_body: Option<JsonObject>) -> PassthroughLoggingPayload {
    PassthroughLoggingPayload {
        url: Some("https://upstream.example/v1/messages".to_owned()),
        request_method: Some("POST".to_owned()),
        request_body,
        response_body: None,
        cost_per_request: None,
    }
}

pub(super) fn chat_input<'call>(
    logging_obj: &'call StreamLoggingObject,
    headers: &'call HeaderMap,
    request_body: &'call JsonObject,
    response_body: &'call JsonObject,
    kwargs: PassThroughLoggingKwargs,
) -> PassthroughChatInput<'call> {
    PassthroughChatInput {
        upstream: upstream(headers),
        response_body,
        logging_obj,
        request_body,
        start_time: start(),
        end_time: end(),
        kwargs,
    }
}

pub(super) fn payload_input<'call>(
    logging_obj: &'call StreamLoggingObject,
    response: &'call mut Json,
    model: &'call str,
    kwargs: PassThroughLoggingKwargs,
) -> ResponsePayloadInput<'call> {
    ResponsePayloadInput {
        response,
        model,
        kwargs,
        start_time: start(),
        end_time: end(),
        logging_obj,
    }
}

pub(super) fn state() -> LoggingState {
    let init = LoggingInit {
        model: "gpt-4o".to_owned(),
        call_type: "pass_through_endpoint".to_owned(),
        start_time: start(),
        litellm_call_id: "call-1".to_owned(),
        function_id: "function-1".to_owned(),
        messages: None,
        stream: Some(false),
        litellm_trace_id: Some("trace-1".to_owned()),
        kwargs: IndexMap::new(),
        litellm_params: IndexMap::new(),
        callback_dynamic_params: IndexMap::new(),
        trusted_callback_vars: Vec::new(),
        standard_built_in_tools_params: IndexMap::new(),
        dynamic_callbacks: DynamicCallbacks::default(),
        applied_guardrails: None,
        request_logging: RequestLoggingFlags::default(),
        callback_policy: CallbackPolicy::default(),
    };
    let mut context = CorrelationContext::default();
    let setup = CorrelationSetup {
        context: &mut context,
        settings: CorrelationSettings::default(),
    };
    LoggingState::new(
        init,
        setup,
        &Redactor::new(16).unwrap(),
        || "generated-trace".to_owned(),
        || Duration::ZERO,
    )
    .unwrap()
}

fn object() -> StreamLoggingObject {
    StreamLoggingObject {
        slot: Mutex::new(Some(StreamLoggingState {
            logging: state(),
            correlation: CorrelationContext::default(),
        })),
        dispatch: Arc::new(NoDispatch),
    }
}

pub(super) fn spent_object() -> StreamLoggingObject {
    StreamLoggingObject {
        slot: Mutex::new(None),
        dispatch: Arc::new(NoDispatch),
    }
}

pub(super) fn details(object: &StreamLoggingObject) -> IndexMap<String, Json> {
    object
        .read(&mut |state| state.model_call_details.clone())
        .unwrap()
}

pub(super) fn state_model(object: &StreamLoggingObject) -> String {
    object.read(&mut |state| state.model.clone()).unwrap()
}

pub(super) fn state_params(object: &StreamLoggingObject) -> IndexMap<String, Json> {
    object
        .read(&mut |state| state.litellm_params.clone())
        .unwrap()
}

struct NoDispatch;

#[async_trait]
impl StreamSuccessDispatch for NoDispatch {
    async fn dispatch_success(&self, _call: StreamSuccessCall<'_>) {}
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TransformCall {
    pub model: String,
    pub status: u16,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
    pub call_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ChunkCall {
    pub model: String,
    pub chunks: Vec<String>,
    pub call_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CostCall {
    pub model: String,
    pub response: Json,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct BuilderCall {
    pub model: String,
    pub response: Json,
    pub start_time: DateTime<FixedOffset>,
    pub end_time: DateTime<FixedOffset>,
    pub details_model: Option<Json>,
    pub cost: Option<Json>,
    pub user: Option<Json>,
}

#[derive(Debug, thiserror::Error)]
#[error("the fake probe failed")]
pub(super) struct ProbeFailed;

#[derive(Clone)]
struct Probe {
    transforms: Arc<Mutex<Vec<TransformCall>>>,
    replies: Arc<Mutex<VecDeque<Result<Json, PassthroughHandlerError>>>>,
}

pub(super) struct FakeProvider {
    name: ProviderId,
    probe: Probe,
    config_models: Mutex<Vec<String>>,
    chunk_calls: Mutex<Vec<ChunkCall>>,
    stream_replies: Mutex<VecDeque<Result<Option<StreamedResponse>, PassthroughHandlerError>>>,
}

impl FakeProvider {
    pub(super) fn new(name: ProviderId) -> Self {
        Self {
            name,
            probe: Probe {
                transforms: Arc::new(Mutex::new(Vec::new())),
                replies: Arc::new(Mutex::new(VecDeque::new())),
            },
            config_models: Mutex::new(Vec::new()),
            chunk_calls: Mutex::new(Vec::new()),
            stream_replies: Mutex::new(VecDeque::new()),
        }
    }

    pub(super) fn transform_ok(&self, response: Json) {
        self.probe
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(Ok(response));
    }

    pub(super) fn transform_fails(&self) {
        self.probe
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(Err(PassthroughHandlerError::leaf(ProbeFailed)));
    }

    pub(super) fn stream(&self, reply: Result<Option<StreamedResponse>, PassthroughHandlerError>) {
        self.stream_replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(reply);
    }

    pub(super) fn stream_fails(&self) {
        self.stream(Err(PassthroughHandlerError::leaf(ProbeFailed)));
    }

    pub(super) fn config_models(&self) -> Vec<String> {
        self.config_models
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn transforms(&self) -> Vec<TransformCall> {
        self.probe
            .transforms
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn chunks(&self) -> Vec<ChunkCall> {
        self.chunk_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

struct FakeConfig {
    probe: Probe,
}

impl BaseConfig for FakeConfig {
    fn transform_response(
        &self,
        upstream: UpstreamChatResponse<'_>,
        model: &str,
        logging_obj: &StreamLoggingObject,
    ) -> Result<Json, PassthroughHandlerError> {
        let content_type = upstream
            .headers
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let call_id = logging_obj.read(&mut |state| state.litellm_call_id.clone());
        self.probe
            .transforms
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(TransformCall {
                model: model.to_owned(),
                status: upstream.status_code,
                content_type,
                body: upstream.body.to_vec(),
                call_id,
            });
        self.probe
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(|| Err(PassthroughHandlerError::leaf(ProbeFailed)))
    }
}

impl PassthroughLoggingProvider for FakeProvider {
    fn llm_provider_name(&self) -> ProviderId {
        self.name
    }

    fn get_provider_config(&self, model: &str) -> Box<dyn BaseConfig> {
        self.config_models
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(model.to_owned());
        Box::new(FakeConfig {
            probe: self.probe.clone(),
        })
    }

    fn build_streaming_response(
        &self,
        all_chunks: &[String],
        logging_obj: &StreamLoggingObject,
        model: &str,
    ) -> Result<Option<StreamedResponse>, PassthroughHandlerError> {
        let reply = self
            .stream_replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(|| Err(PassthroughHandlerError::leaf(ProbeFailed)));
        self.chunk_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ChunkCall {
                model: model.to_owned(),
                chunks: all_chunks.to_vec(),
                call_id: logging_obj.read(&mut |state| state.litellm_call_id.clone()),
            });
        reply
    }
}

#[derive(Default)]
pub(super) struct FakeCost {
    replies: Mutex<VecDeque<Result<Decimal, PassthroughHandlerError>>>,
    calls: Mutex<Vec<CostCall>>,
}

impl FakeCost {
    pub(super) fn costs(&self, amount: Decimal) {
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(Ok(amount));
    }

    pub(super) fn fails(&self) {
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(Err(PassthroughHandlerError::leaf(ProbeFailed)));
    }

    pub(super) fn calls(&self) -> Vec<CostCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ResponseCost for FakeCost {
    fn completion_cost(
        &self,
        response: &Json,
        model: &str,
    ) -> Result<Decimal, PassthroughHandlerError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(CostCall {
                model: model.to_owned(),
                response: response.clone(),
            });
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(|| Err(PassthroughHandlerError::leaf(ProbeFailed)))
    }
}

#[derive(Default)]
pub(super) struct RecordingBuilder {
    payload: Option<StandardLoggingPayload>,
    calls: Mutex<Vec<BuilderCall>>,
}

impl RecordingBuilder {
    pub(super) fn returning(payload: StandardLoggingPayload) -> Self {
        Self {
            payload: Some(payload),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn calls(&self) -> Vec<BuilderCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl StandardPayloadBuilder for RecordingBuilder {
    fn build(
        &self,
        state: &LoggingState,
        response: &Json,
        start_time: DateTime<FixedOffset>,
        end_time: DateTime<FixedOffset>,
    ) -> Option<StandardLoggingPayload> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(BuilderCall {
                model: state.model.clone(),
                response: response.clone(),
                start_time,
                end_time,
                details_model: state.model_call_details.get("model").cloned(),
                cost: state.model_call_details.get("response_cost").cloned(),
                user: state_user(state),
            });
        self.payload.clone()
    }
}

fn state_user(state: &LoggingState) -> Option<Json> {
    let Some(Json::Object(params)) = state.litellm_params.get("proxy_server_request") else {
        return None;
    };
    let Some(Json::Object(request)) = params.get("body") else {
        return None;
    };
    request.get("user").cloned()
}

pub(super) struct Harness {
    pub provider: Arc<FakeProvider>,
    pub cost: Arc<FakeCost>,
    pub builder: Arc<RecordingBuilder>,
    pub handler: BasePassthroughLoggingHandler,
    pub logging: StreamLoggingObject,
}

pub(super) fn harness() -> Harness {
    build(ProviderId::Anthropic, RecordingBuilder::default())
}

pub(super) fn harness_named(name: ProviderId) -> Harness {
    build(name, RecordingBuilder::default())
}

pub(super) fn harness_returning(payload: StandardLoggingPayload) -> Harness {
    build(ProviderId::Anthropic, RecordingBuilder::returning(payload))
}

fn build(name: ProviderId, builder: RecordingBuilder) -> Harness {
    let provider = Arc::new(FakeProvider::new(name));
    let cost = Arc::new(FakeCost::default());
    let builder = Arc::new(builder);
    let handler = BasePassthroughLoggingHandler {
        provider: Arc::clone(&provider),
        cost: Arc::clone(&cost),
        payload_builder: Arc::clone(&builder),
        request_headers: Settings::default(),
        validate_end_user_id_in_db: false,
    };
    Harness {
        provider,
        cost,
        builder,
        handler,
        logging: object(),
    }
}

#[derive(Clone, Default)]
pub(super) struct Captured(pub Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub(super) fn text(&self) -> String {
        let bytes = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        String::from_utf8(bytes).unwrap()
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

pub(super) fn capture_logs(level: Level) -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(level)
        .with_writer(move || writer.clone())
        .finish();
    (captured, set_default(subscriber))
}
