use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::sections::logging_privacy::Settings;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;

use crate::callback_logger::CallbackLogger;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::exporter_options::failure_counter::CallbackFailureCounter;
use crate::exporter_options::redaction::ResponseStamp;
use crate::fields::payload::StandardLoggingPayload;
use crate::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup, SecretRedaction,
};
use crate::logging_state::types::{CallbackPolicy, DynamicCallbacks, RequestLoggingFlags};
use crate::logging_state::{LoggingInit, LoggingState};
use crate::registry::CallbackRegistry;
use crate::registry::Registration;
use crate::registry::identity::CallbackIdentity;
use crate::registry::phases::CallbackList;
use crate::registry::sink::CallbackSink;

use super::super::handler::{SuccessHandler, async_success_handler, success_handler};
use super::super::ports::{StandardPayloadBuilder, StreamAssembler, SuccessClock};
use super::super::types::{SuccessMode, SuccessRequest, SuccessResult};

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct LoggedCall {
    pub event: String,
    pub stream: bool,
    pub model: String,
    pub messages: Option<Json>,
    pub response: Option<Json>,
    pub litellm_metadata: IndexMap<String, Json>,
}

#[derive(Default)]
pub(super) struct CallLog(Mutex<Vec<LoggedCall>>);

impl CallLog {
    pub(super) fn record(&self, call: LoggedCall) {
        self.0.lock().unwrap().push(call);
    }

    pub(super) fn calls(&self) -> Vec<LoggedCall> {
        self.0.lock().unwrap().clone()
    }

    pub(super) fn events(&self) -> Vec<String> {
        self.calls().into_iter().map(|call| call.event).collect()
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct MessageFlags {
    pub turn_off_message_logging: bool,
    pub redacts_messages_itself: bool,
}

pub(super) struct FakeLogger {
    pub name: String,
    pub calls: Arc<CallLog>,
    pub flags: MessageFlags,
    pub message_logging: bool,
    pub fail: bool,
}

impl FakeLogger {
    pub(super) fn new(name: &str, calls: &Arc<CallLog>) -> Self {
        Self {
            name: name.to_owned(),
            calls: Arc::clone(calls),
            flags: MessageFlags::default(),
            message_logging: true,
            fail: false,
        }
    }

    fn record(&self, name: &str, event: &LoggerEvent) -> Result<(), LoggerError> {
        self.calls.record(LoggedCall {
            event: name.to_owned(),
            stream: event.stream,
            model: event.payload.model.clone(),
            messages: event.payload.messages.clone(),
            response: event.payload.response.clone(),
            litellm_metadata: event.litellm_metadata.as_ref().clone(),
        });
        if self.fail {
            return Err(LoggerError::Delivery {
                callback: self.name.clone(),
                message: "the transport is down".to_owned(),
            });
        }
        Ok(())
    }
}

#[async_trait]
impl CallbackLogger for FakeLogger {
    fn callback_name(&self) -> &str {
        &self.name
    }

    fn turn_off_message_logging(&self) -> bool {
        self.flags.turn_off_message_logging
    }

    fn redacts_messages_itself(&self) -> bool {
        self.flags.redacts_messages_itself
    }

    fn message_logging(&self) -> bool {
        self.message_logging
    }

    fn log_success_event(&self, event: &LoggerEvent) -> Result<(), LoggerError> {
        self.record("log_success_event", event)
    }

    fn log_stream_event(&self, event: &LoggerEvent) -> Result<(), LoggerError> {
        self.record("log_stream_event", event)
    }

    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.record("async_log_success_event", &event)
    }

    async fn async_log_stream_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.record("async_log_stream_event", &event)
    }
}

pub(super) struct FakeBuilder {
    pub payload: Option<StandardLoggingPayload>,
    pub builds: Mutex<usize>,
}

impl FakeBuilder {
    pub(super) fn returning(payload: Option<StandardLoggingPayload>) -> Self {
        Self {
            payload,
            builds: Mutex::new(0),
        }
    }

    pub(super) fn builds(&self) -> usize {
        *self.builds.lock().unwrap()
    }
}

impl StandardPayloadBuilder for FakeBuilder {
    fn build(
        &self,
        _state: &LoggingState,
        _response: &Json,
        _start_time: DateTime<FixedOffset>,
        _end_time: DateTime<FixedOffset>,
    ) -> Option<StandardLoggingPayload> {
        let mut builds = self.builds.lock().unwrap();
        *builds = builds.saturating_add(1);
        drop(builds);
        self.payload.clone()
    }
}

pub(super) struct FakeAssembler {
    pub complete: Option<Json>,
}

impl StreamAssembler for FakeAssembler {
    fn assemble(
        &self,
        state: &LoggingState,
        result: &SuccessResult,
        _mode: SuccessMode,
    ) -> Option<Json> {
        if state.stream != Some(true) {
            return None;
        }
        match result {
            SuccessResult::Chunk(_) => self.complete.clone(),
            SuccessResult::Chat(_) | SuccessResult::Embeddings(_) | SuccessResult::Other(_) => {
                Some(result.response().clone())
            }
        }
    }
}

pub(super) struct FixedClock {
    pub now: DateTime<FixedOffset>,
}

impl SuccessClock for FixedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        self.now
    }
}

#[derive(Default)]
pub(super) struct RecordingCounter(Mutex<Vec<String>>);

impl RecordingCounter {
    pub(super) fn calls(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

impl CallbackFailureCounter for RecordingCounter {
    fn increment_callback_logging_failure(&self, callback_name: &str) {
        self.0.lock().unwrap().push(callback_name.to_owned());
    }
}

pub(super) fn stamp() -> ResponseStamp {
    ResponseStamp {
        id: "chatcmpl-placeholder".to_owned(),
        created: 1_704_067_200,
    }
}

fn logger_handle(logger: Arc<FakeLogger>) -> Arc<dyn CallbackLogger> {
    logger
}

fn counter_handle(counter: Arc<RecordingCounter>) -> Arc<dyn CallbackFailureCounter + Send + Sync> {
    counter
}

pub(super) fn register_logger(
    registry: &mut CallbackRegistry,
    logger: &Arc<FakeLogger>,
    list: CallbackList,
) {
    let identity = CallbackIdentity {
        class_name: logger.name.clone(),
        attributes: IndexMap::new(),
    };
    let sink = CallbackSink::Logger(logger_handle(Arc::clone(logger)));
    let registration = registry.register(identity, sink, list);
    assert_ne!(registration, Registration::SkippedCapacity);
}

pub(super) fn time(text: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(text).unwrap()
}

fn init(
    stream: Option<bool>,
    litellm_trace_id: Option<String>,
    kwargs: IndexMap<String, Json>,
) -> LoggingInit {
    LoggingInit {
        model: "gpt-4o".to_owned(),
        call_type: "completion".to_owned(),
        start_time: time("2024-01-01T00:00:00+00:00"),
        litellm_call_id: "call-1".to_owned(),
        function_id: "function-1".to_owned(),
        messages: None,
        stream,
        litellm_trace_id,
        kwargs,
        litellm_params: IndexMap::new(),
        callback_dynamic_params: IndexMap::new(),
        trusted_callback_vars: Vec::new(),
        standard_built_in_tools_params: IndexMap::new(),
        dynamic_callbacks: DynamicCallbacks::default(),
        applied_guardrails: None,
        request_logging: RequestLoggingFlags::default(),
        callback_policy: CallbackPolicy::default(),
    }
}

pub(super) fn state(stream: Option<bool>) -> LoggingState {
    new_state(
        init(stream, Some("trace-1".to_owned()), IndexMap::new()),
        &mut CorrelationContext::default(),
        CorrelationSettings::default(),
    )
}

pub(super) fn correlated_state(
    stream: Option<bool>,
    context: &mut CorrelationContext,
) -> LoggingState {
    let mut kwargs = IndexMap::new();
    kwargs.insert(
        "litellm_session_id".to_owned(),
        Json::Text("inner-session".to_owned()),
    );
    let settings = CorrelationSettings {
        supports_correlation_logging: true,
        request_correlation_in_logs: true,
        secret_redaction: SecretRedaction::Enabled,
    };
    new_state(
        init(stream, Some("inner-trace".to_owned()), kwargs),
        context,
        settings,
    )
}

pub(super) fn new_state(
    init: LoggingInit,
    context: &mut CorrelationContext,
    settings: CorrelationSettings,
) -> LoggingState {
    LoggingState::new(
        init,
        CorrelationSetup { context, settings },
        &Redactor::new(16).unwrap(),
        || "generated-trace".to_owned(),
        || Duration::ZERO,
    )
    .unwrap()
}

pub(super) struct Fixture {
    pub registry: CallbackRegistry,
    pub builder: FakeBuilder,
    pub assembler: FakeAssembler,
    pub clock: FixedClock,
    pub counters: Vec<Arc<dyn CallbackFailureCounter + Send + Sync>>,
    pub counter_calls: Vec<Arc<RecordingCounter>>,
    pub excluded_fields: Vec<String>,
    pub known: Box<dyn Fn(&str) -> bool + Send + Sync>,
    pub stamp: ResponseStamp,
    pub privacy: Settings,
}

impl Fixture {
    pub(super) fn new() -> Self {
        let (counters, counter_calls) = recording_counters();
        Self {
            registry: CallbackRegistry::new(10),
            builder: FakeBuilder::returning(Some(payload())),
            assembler: FakeAssembler { complete: None },
            clock: FixedClock {
                now: time("2024-01-01T00:00:02+00:00"),
            },
            counters,
            counter_calls,
            excluded_fields: Vec::new(),
            known: Box::new(|_name: &str| false),
            stamp: stamp(),
            privacy: Settings::default(),
        }
    }

    pub(super) fn add(&mut self, logger: &Arc<FakeLogger>) {
        register_logger(&mut self.registry, logger, CallbackList::Callbacks);
    }

    pub(super) fn set_known(&mut self, names: Vec<String>) {
        self.known = Box::new(move |name: &str| names.iter().any(|known| known == name));
    }

    pub(super) fn handler(&self) -> SuccessHandler<'_> {
        SuccessHandler {
            registry: &self.registry,
            payload_builder: &self.builder,
            stream_assembler: &self.assembler,
            clock: &self.clock,
            failure_counters: &self.counters,
            excluded_fields: &self.excluded_fields,
            known_callback: self.known.as_ref(),
            stamp: &self.stamp,
            privacy: self.privacy,
        }
    }
}

impl Default for Fixture {
    fn default() -> Self {
        Self::new()
    }
}

fn recording_counters() -> (
    Vec<Arc<dyn CallbackFailureCounter + Send + Sync>>,
    Vec<Arc<RecordingCounter>>,
) {
    let first = Arc::new(RecordingCounter::default());
    let second = Arc::new(RecordingCounter::default());
    (
        vec![
            counter_handle(Arc::clone(&first)),
            counter_handle(Arc::clone(&second)),
        ],
        vec![first, second],
    )
}

pub(super) fn json_object(fields: &[(&str, Json)]) -> Json {
    let mut object = IndexMap::new();
    for (name, value) in fields {
        object.insert((*name).to_owned(), value.clone());
    }
    Json::Object(object)
}

pub(super) fn chat_result() -> SuccessResult {
    SuccessResult::Chat(json_object(&[
        ("id", Json::Text("chatcmpl-1".to_owned())),
        ("object", Json::Text("chat.completion".to_owned())),
    ]))
}

pub(super) fn chunk_result() -> SuccessResult {
    SuccessResult::Chunk(json_object(&[
        ("id", Json::Text("chatcmpl-chunk".to_owned())),
        ("object", Json::Text("chat.completion.chunk".to_owned())),
    ]))
}

pub(super) fn request(result: SuccessResult) -> SuccessRequest {
    build_request(
        result,
        time("2024-01-01T00:00:00+00:00"),
        time("2024-01-01T00:00:01+00:00"),
        Some(payload()),
    )
}

fn build_request(
    result: SuccessResult,
    start_time: DateTime<FixedOffset>,
    end_time: DateTime<FixedOffset>,
    standard_logging_object: Option<StandardLoggingPayload>,
) -> SuccessRequest {
    SuccessRequest {
        result,
        start_time: Some(start_time),
        end_time: Some(end_time),
        cache_hit: Some(false),
        prefer_async_handlers: false,
        standard_logging_object,
        guardrail_snapshot: None,
    }
}

pub(super) fn bare_request(result: SuccessResult) -> SuccessRequest {
    let mut request = request(result);
    request.start_time = None;
    request.end_time = None;
    request.cache_hit = None;
    request.standard_logging_object = None;
    request
}

pub(super) fn correlation() -> CorrelationContext {
    CorrelationContext::default()
}

pub(super) async fn run_once(
    path: &str,
    handler: &SuccessHandler<'_>,
    state: &mut LoggingState,
    correlation: &mut CorrelationContext,
    request: &SuccessRequest,
) {
    if path == "sync" {
        success_handler(handler, state, correlation, request);
    } else {
        async_success_handler(handler, state, correlation, request).await;
    }
}

pub(super) fn parse(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}
