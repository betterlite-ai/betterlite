use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::sections::logging_privacy::Settings;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use crate::callback_logger::CallbackLogger;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::exporter_options::redaction::ResponseStamp;
use crate::fields::payload::StandardLoggingPayload;
use crate::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use crate::logging_state::types::{CallbackPolicy, DynamicCallbacks, RequestLoggingFlags};
use crate::logging_state::{LoggingInit, LoggingState};
use crate::logging_success::handler::SuccessHandler;
use crate::logging_success::ports::{StandardPayloadBuilder, StreamAssembler, SuccessClock};
use crate::logging_success::types::{SuccessMode, SuccessResult};
use crate::passthrough_logging_state::dispatch::{
    PassThroughLogCall, PassThroughLogDispatch, PassThroughLoggingKwargs,
};
use crate::passthrough_logging_state::response::PassThroughResponse;
use crate::registry::CallbackRegistry;
use crate::registry::Registration;
use crate::registry::identity::CallbackIdentity;
use crate::registry::phases::CallbackList;
use crate::registry::sink::CallbackSink;

const PAYLOAD: &str = r#"{
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
    serde_json::from_str(PAYLOAD).expect("valid fixture payload")
}

pub(super) fn time(text: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(text).expect("valid fixture time")
}

pub(super) fn chat_json() -> Json {
    let mut object = IndexMap::new();
    object.insert("id".to_owned(), Json::Text("chatcmpl-1".to_owned()));
    object.insert(
        "object".to_owned(),
        Json::Text("chat.completion".to_owned()),
    );
    Json::Object(object)
}

pub(super) fn chat_response() -> PassThroughResponse {
    PassThroughResponse::ModelResponse(chat_json())
}

#[derive(Default)]
pub(super) struct RecordedCalls(Mutex<Vec<String>>);

impl RecordedCalls {
    fn record(&self, name: &str) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(name.to_owned());
    }

    pub(super) fn names(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

pub(super) struct RecordingLogger {
    name: String,
    calls: Arc<RecordedCalls>,
}

#[async_trait]
impl CallbackLogger for RecordingLogger {
    fn callback_name(&self) -> &str {
        &self.name
    }

    fn log_success_event(&self, _event: &LoggerEvent) -> Result<(), LoggerError> {
        self.calls.record("log_success_event");
        Ok(())
    }

    fn log_stream_event(&self, _event: &LoggerEvent) -> Result<(), LoggerError> {
        self.calls.record("log_stream_event");
        Ok(())
    }

    async fn async_log_success_event(&self, _event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.calls.record("async_log_success_event");
        Ok(())
    }

    async fn async_log_stream_event(&self, _event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.calls.record("async_log_stream_event");
        Ok(())
    }
}

pub(super) struct FakeBuilder {
    payload: Option<StandardLoggingPayload>,
}

impl StandardPayloadBuilder for FakeBuilder {
    fn build(
        &self,
        _state: &LoggingState,
        _response: &Json,
        _start_time: DateTime<FixedOffset>,
        _end_time: DateTime<FixedOffset>,
    ) -> Option<StandardLoggingPayload> {
        self.payload.clone()
    }
}

pub(super) struct FakeAssembler;

impl StreamAssembler for FakeAssembler {
    fn assemble(
        &self,
        _state: &LoggingState,
        _result: &SuccessResult,
        _mode: SuccessMode,
    ) -> Option<Json> {
        None
    }
}

pub(super) struct FixedClock {
    now: DateTime<FixedOffset>,
}

impl SuccessClock for FixedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        self.now
    }
}

pub(super) struct Fixture {
    registry: CallbackRegistry,
    builder: FakeBuilder,
    assembler: FakeAssembler,
    clock: FixedClock,
    excluded_fields: Vec<String>,
    known: Box<dyn Fn(&str) -> bool + Send + Sync>,
    stamp: ResponseStamp,
    privacy: Settings,
    pub(super) calls: Arc<RecordedCalls>,
}

impl Fixture {
    pub(super) fn new(payload: Option<StandardLoggingPayload>) -> Self {
        let mut registry = CallbackRegistry::new(10);
        let calls = Arc::new(RecordedCalls::default());
        let logger = Arc::new(RecordingLogger {
            name: "recorder".to_owned(),
            calls: Arc::clone(&calls),
        });
        let identity = CallbackIdentity {
            class_name: logger.name.clone(),
            attributes: IndexMap::new(),
        };
        let sink = CallbackSink::Logger(logger);
        let registration = registry.register(identity, sink, CallbackList::Callbacks);
        assert_ne!(registration, Registration::SkippedCapacity);
        Self {
            registry,
            builder: FakeBuilder { payload },
            assembler: FakeAssembler,
            clock: FixedClock {
                now: time("2024-01-01T00:00:02+00:00"),
            },
            excluded_fields: Vec::new(),
            known: Box::new(|_name: &str| false),
            stamp: ResponseStamp {
                id: "chatcmpl-placeholder".to_owned(),
                created: 1_704_067_200,
            },
            privacy: Settings::default(),
            calls,
        }
    }

    pub(super) fn handler(&self) -> SuccessHandler<'_> {
        SuccessHandler {
            registry: &self.registry,
            payload_builder: &self.builder,
            stream_assembler: &self.assembler,
            clock: &self.clock,
            failure_counters: &[],
            excluded_fields: &self.excluded_fields,
            known_callback: self.known.as_ref(),
            stamp: &self.stamp,
            privacy: self.privacy,
        }
    }
}

pub(super) fn logging_state(call_type: &str, context: &mut CorrelationContext) -> LoggingState {
    let init = LoggingInit {
        model: "gpt-4o".to_owned(),
        call_type: call_type.to_owned(),
        start_time: time("2024-01-01T00:00:00+00:00"),
        litellm_call_id: "call-1".to_owned(),
        function_id: "function-1".to_owned(),
        messages: None,
        stream: None,
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
    let setup = CorrelationSetup {
        context,
        settings: CorrelationSettings::default(),
    };
    LoggingState::new(
        init,
        setup,
        &Redactor::new(16).expect("valid redactor"),
        || "generated-trace".to_owned(),
        || Duration::ZERO,
    )
    .expect("valid logging state")
}

pub(super) fn call<'call>(
    handler: &'call SuccessHandler<'call>,
    state: &'call mut LoggingState,
    correlation: &'call mut CorrelationContext,
) -> PassThroughLogCall<'call> {
    PassThroughLogCall {
        handler,
        state,
        correlation,
        response: None,
        result: "",
        start_time: time("2024-01-01T00:00:00+00:00"),
        end_time: time("2024-01-01T00:00:01+00:00"),
        cache_hit: false,
        kwargs: PassThroughLoggingKwargs::default(),
    }
}

#[derive(Clone)]
pub(super) struct RecordedDispatch {
    pub(super) response: Option<PassThroughResponse>,
    pub(super) result: String,
    pub(super) start_time: DateTime<FixedOffset>,
    pub(super) end_time: DateTime<FixedOffset>,
    pub(super) cache_hit: bool,
    pub(super) kwargs: PassThroughLoggingKwargs,
}

#[derive(Default)]
pub(super) struct RecordingDispatch {
    calls: Mutex<Vec<RecordedDispatch>>,
}

impl RecordingDispatch {
    pub(super) fn recorded(&self) -> Vec<RecordedDispatch> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl PassThroughLogDispatch for RecordingDispatch {
    async fn dispatch(&self, call: PassThroughLogCall<'_>) {
        let recorded = RecordedDispatch {
            response: call.response,
            result: call.result.to_owned(),
            start_time: call.start_time,
            end_time: call.end_time,
            cache_hit: call.cache_hit,
            kwargs: call.kwargs,
        };
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(recorded);
    }
}

#[derive(Clone, Default)]
pub(super) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub(super) fn text(&self) -> String {
        let bytes = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        String::from_utf8(bytes).expect("captured logs are UTF-8")
    }
}

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for Captured {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

pub(super) fn capture_logs() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(tracing::Level::WARN)
        .with_writer(move || writer.clone())
        .finish();
    (captured, set_default(subscriber))
}

// the fuzz target decodes arbitrary bytes the same way; from_utf8_lossy is not allowed in this crate
pub(super) fn lossy_text(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len());
    for chunk in bytes.utf8_chunks() {
        text.push_str(chunk.valid());
        if !chunk.invalid().is_empty() {
            text.push(char::REPLACEMENT_CHARACTER);
        }
    }
    text
}
