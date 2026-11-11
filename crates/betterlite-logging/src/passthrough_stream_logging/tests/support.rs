use std::io::{Result as IoResult, Write};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{ThreadId, current};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::sections::logging_privacy::Settings;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt;
use tracing_subscriber::fmt::MakeWriter;

use super::super::build::StreamLoggingInput;
use super::super::object::{
    StreamLoggingCallbacks, StreamLoggingObject, StreamLoggingState, StreamSuccessCall,
    StreamSuccessDispatch,
};
use super::super::router::{AnthropicFrameProbe, StreamLoggingRouter};
use super::super::tinyfish::{TinyfishCollectedChunks, TinyfishStreamBilling};
use super::super::types::{
    CollectedChunksCall, CollectedChunksHandler, EndpointType, HandlerError, HandlerOutcome,
    ProviderChunkHandlers,
};
use crate::callback_logger::CallbackLogger;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::exporter_options::failure_counter::CallbackFailureCounter;
use crate::exporter_options::redaction::ResponseStamp;
use crate::fields::payload::StandardLoggingPayload;
use crate::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use crate::logging_state::types::{CallbackPolicy, DynamicCallbacks, RequestLoggingFlags};
use crate::logging_state::{LoggingInit, LoggingState};
use crate::logging_success::ports::{StandardPayloadBuilder, StreamAssembler, SuccessClock};
use crate::logging_success::types::{SuccessMode, SuccessResult};
use crate::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use crate::passthrough_logging_state::logger::PassThroughEndpointLogging;
use crate::registry::CallbackRegistry;
use crate::registry::Registration;
use crate::registry::identity::CallbackIdentity;
use crate::registry::phases::CallbackList;
use crate::registry::sink::CallbackSink;

pub(super) const PASS_THROUGH: &str = "pass_through_endpoint";

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

pub(super) fn time(text: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(text).unwrap()
}

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("valid JSON literal")
}

pub(super) fn state(call_type: &str, details: &[(&str, Json)]) -> LoggingState {
    let init = LoggingInit {
        model: "gpt-4o".to_owned(),
        call_type: call_type.to_owned(),
        start_time: time("2024-01-01T00:00:00+00:00"),
        litellm_call_id: "call-1".to_owned(),
        function_id: "function-1".to_owned(),
        messages: None,
        stream: Some(true),
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
    let mut logging = LoggingState::new(
        init,
        setup,
        &Redactor::new(16).expect("valid redactor"),
        || "generated-trace".to_owned(),
        || Duration::ZERO,
    )
    .expect("valid logging state");
    for (key, value) in details {
        logging
            .model_call_details
            .insert((*key).to_owned(), value.clone());
    }
    logging
}

pub(super) fn object(
    state: LoggingState,
    dispatch: Arc<dyn StreamSuccessDispatch>,
) -> Arc<StreamLoggingObject> {
    Arc::new(StreamLoggingObject {
        slot: Mutex::new(Some(StreamLoggingState {
            logging: state,
            correlation: CorrelationContext::default(),
        })),
        dispatch,
    })
}

pub(super) fn passthrough() -> Arc<PassThroughEndpointLogging> {
    Arc::new(PassThroughEndpointLogging {
        injected_log_dispatch: None,
    })
}

pub(super) fn stream_input(
    endpoint_type: EndpointType,
) -> (Arc<StreamLoggingObject>, StreamLoggingInput) {
    let logging_obj = object(
        state(PASS_THROUGH, &[]),
        Arc::new(RecordingDispatch::default()),
    );
    let input = input(
        Arc::clone(&logging_obj),
        Arc::clone(&handlers().providers),
        endpoint_type,
    );
    (logging_obj, input)
}

pub(super) fn input(
    logging_obj: Arc<StreamLoggingObject>,
    providers: Arc<ProviderChunkHandlers>,
    endpoint_type: EndpointType,
) -> StreamLoggingInput {
    StreamLoggingInput {
        logging_obj,
        passthrough: passthrough(),
        providers,
        url_route: "/v1/messages".to_owned(),
        request_body: Some(Json::Object(IndexMap::new())),
        endpoint_type,
        start_time: time("2024-01-01T00:00:00+00:00"),
        end_time: time("2024-01-01T00:00:02+00:00"),
        raw_bytes: Vec::new(),
        model: None,
    }
}

#[derive(Clone, Debug)]
pub(super) struct RecordedCall {
    pub handler: &'static str,
    pub url_route: String,
    pub request_body: Option<Json>,
    pub endpoint_type: EndpointType,
    pub start_time: DateTime<FixedOffset>,
    pub end_time: DateTime<FixedOffset>,
    pub lines: Vec<String>,
    pub model: Option<String>,
    pub thread: ThreadId,
}

pub(super) struct FakeHandler {
    name: &'static str,
    reply: Mutex<Option<Result<HandlerOutcome, HandlerError>>>,
    calls: Mutex<Vec<RecordedCall>>,
}

impl FakeHandler {
    pub(super) fn returning(
        name: &'static str,
        reply: Result<HandlerOutcome, HandlerError>,
    ) -> Self {
        Self {
            name,
            reply: Mutex::new(Some(reply)),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn calls(&self) -> Vec<RecordedCall> {
        self.calls.lock().unwrap().clone()
    }
}

impl CollectedChunksHandler for FakeHandler {
    fn handle(&self, call: &CollectedChunksCall<'_>) -> Result<HandlerOutcome, HandlerError> {
        let recorded = RecordedCall {
            handler: self.name,
            url_route: call.url_route.to_owned(),
            request_body: call.request_body.cloned(),
            endpoint_type: call.endpoint_type,
            start_time: call.start_time,
            end_time: call.end_time,
            lines: call.lines.to_vec(),
            model: call.model.map(str::to_owned),
            thread: current().id(),
        };
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(recorded);
        next_reply(&self.reply)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the fake handler was called more than once")]
pub(super) struct FakeCalledTwice;

fn next_reply(
    slot: &Mutex<Option<Result<HandlerOutcome, HandlerError>>>,
) -> Result<HandlerOutcome, HandlerError> {
    slot.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take()
        .unwrap_or_else(|| Err(HandlerError::leaf(FakeCalledTwice)))
}

pub(super) struct PanickingHandler;

fn handler_panic() -> HandlerOutcome {
    panic!("the collected chunks handler panicked");
}

impl CollectedChunksHandler for PanickingHandler {
    fn handle(&self, _call: &CollectedChunksCall<'_>) -> Result<HandlerOutcome, HandlerError> {
        Ok(handler_panic())
    }
}

pub(super) struct HandlerSet {
    pub anthropic: Arc<FakeHandler>,
    pub vertex: Arc<FakeHandler>,
    pub gemini: Arc<FakeHandler>,
    pub openai: Arc<FakeHandler>,
    pub providers: Arc<ProviderChunkHandlers>,
}

pub(super) fn handlers() -> HandlerSet {
    let anthropic = Arc::new(FakeHandler::returning(
        "anthropic",
        Ok(HandlerOutcome::default()),
    ));
    let vertex = Arc::new(FakeHandler::returning(
        "vertex",
        Ok(HandlerOutcome::default()),
    ));
    let gemini = Arc::new(FakeHandler::returning(
        "gemini",
        Ok(HandlerOutcome::default()),
    ));
    let openai = Arc::new(FakeHandler::returning(
        "openai",
        Ok(HandlerOutcome::default()),
    ));
    let anthropic_provider = Arc::clone(&anthropic);
    let vertex_provider = Arc::clone(&vertex);
    let gemini_provider = Arc::clone(&gemini);
    let openai_provider = Arc::clone(&openai);
    let providers = Arc::new(ProviderChunkHandlers {
        anthropic: anthropic_provider,
        vertex: vertex_provider,
        gemini: gemini_provider,
        openai: openai_provider,
    });
    HandlerSet {
        anthropic,
        vertex,
        gemini,
        openai,
        providers,
    }
}

pub(super) fn providers_with(
    handler: &Arc<dyn CollectedChunksHandler>,
) -> Arc<ProviderChunkHandlers> {
    Arc::new(ProviderChunkHandlers {
        anthropic: Arc::clone(handler),
        vertex: Arc::clone(handler),
        gemini: Arc::clone(handler),
        openai: Arc::clone(handler),
    })
}

pub(super) fn dispatch_handle(dispatch: Arc<RecordingDispatch>) -> Arc<dyn StreamSuccessDispatch> {
    dispatch
}

pub(super) fn billing_handle(tinyfish: Arc<FakeTinyfish>) -> Arc<dyn TinyfishStreamBilling> {
    tinyfish
}

pub(super) fn handler_handle(handler: Arc<FakeHandler>) -> Arc<dyn CollectedChunksHandler> {
    handler
}

pub(super) async fn route_stream(
    input: StreamLoggingInput,
    frames: Arc<dyn AnthropicFrameProbe>,
    tinyfish: Arc<dyn TinyfishStreamBilling>,
) {
    StreamLoggingRouter {
        providers: Arc::clone(&input.providers),
        frames,
        tinyfish,
    }
    .route_streaming_logging_to_handler(input)
    .await;
}

pub(super) async fn route_default(input: StreamLoggingInput, frames: Arc<dyn AnthropicFrameProbe>) {
    route_stream(input, frames, Arc::new(FakeTinyfish::default())).await;
}

pub(super) async fn route_anthropic(
    raw: &[u8],
    stream: ProbeStream,
) -> (Arc<StreamLoggingObject>, Arc<FakeProbe>) {
    let (logging_obj, mut input) = stream_input(EndpointType::Anthropic);
    input.raw_bytes = vec![raw.to_vec()];
    let fake = probe(stream);
    let frames = Arc::clone(&fake);
    route_default(input, frames).await;
    (logging_obj, fake)
}

// The fuzz target splits the same way, so a crash input replays with its chunk boundaries.
pub(super) fn corpus_chunks(bytes: &[u8]) -> Vec<Vec<u8>> {
    let Some((&first, rest)) = bytes.split_first() else {
        return Vec::new();
    };
    let stride = usize::from(first).rem_euclid(7).saturating_add(1);
    rest.chunks(stride).map(<[u8]>::to_vec).collect()
}

#[derive(Default)]
pub(super) struct RecordingDispatch {
    calls: Mutex<Vec<RecordedDispatch>>,
}

impl RecordingDispatch {
    pub(super) fn calls(&self) -> Vec<RecordedDispatch> {
        self.calls.lock().unwrap().clone()
    }
}

#[derive(Clone, Debug)]
pub(super) struct RecordedDispatch {
    pub result: Json,
    pub start_time: DateTime<FixedOffset>,
    pub end_time: DateTime<FixedOffset>,
    pub cache_hit: bool,
    pub kwargs: PassThroughLoggingKwargs,
}

#[async_trait]
impl StreamSuccessDispatch for RecordingDispatch {
    async fn dispatch_success(&self, call: StreamSuccessCall<'_>) {
        let recorded = RecordedDispatch {
            result: call.result.response().clone(),
            start_time: call.start_time,
            end_time: call.end_time,
            cache_hit: call.cache_hit,
            kwargs: call.kwargs.clone(),
        };
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(recorded);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ProbeStream {
    Complete,
    NoMessageStop,
    ProviderError,
}

pub(super) struct FakeProbe {
    pub stop: bool,
    pub provider_error: bool,
    pub stop_inputs: Mutex<Vec<Vec<u8>>>,
    pub error_inputs: Mutex<Vec<Vec<u8>>>,
}

pub(super) fn probe(stream: ProbeStream) -> Arc<FakeProbe> {
    let stop = matches!(stream, ProbeStream::Complete | ProbeStream::ProviderError);
    let provider_error = stream == ProbeStream::ProviderError;
    Arc::new(FakeProbe {
        stop,
        provider_error,
        stop_inputs: Mutex::new(Vec::new()),
        error_inputs: Mutex::new(Vec::new()),
    })
}

impl AnthropicFrameProbe for FakeProbe {
    fn is_message_stop(&self, frames: &[u8]) -> bool {
        self.stop_inputs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(frames.to_vec());
        self.stop
    }

    fn is_provider_error(&self, frames: &[u8]) -> bool {
        self.error_inputs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(frames.to_vec());
        self.provider_error
    }
}

#[derive(Default)]
pub(super) struct FakeTinyfish {
    pub poller: bool,
    pub run_id: Option<String>,
    pub poller_checks: Mutex<usize>,
    pub scanned: Mutex<Vec<Vec<u8>>>,
    pub started: Mutex<Vec<(String, DateTime<FixedOffset>)>>,
    pub handler_calls: Mutex<Vec<Vec<String>>>,
    pub reply: Mutex<Option<Result<HandlerOutcome, HandlerError>>>,
}

impl FakeTinyfish {
    pub(super) fn for_run(poller: bool, run_id: Option<String>) -> Self {
        Self {
            poller,
            run_id,
            poller_checks: Mutex::new(0),
            scanned: Mutex::new(Vec::new()),
            started: Mutex::new(Vec::new()),
            handler_calls: Mutex::new(Vec::new()),
            reply: Mutex::new(None),
        }
    }

    pub(super) fn with_reply(mut self, reply: Result<HandlerOutcome, HandlerError>) -> Self {
        self.reply = Mutex::new(Some(reply));
        self
    }
}

#[async_trait]
impl TinyfishStreamBilling for FakeTinyfish {
    fn sse_poller_spawned(&self, _logging_obj: &StreamLoggingObject) -> bool {
        let mut checks = self.poller_checks.lock().unwrap();
        *checks = checks.saturating_add(1);
        drop(checks);
        self.poller
    }

    fn run_id_from_sse_frames(&self, frames: &[u8]) -> Option<String> {
        self.scanned.lock().unwrap().push(frames.to_vec());
        self.run_id.clone()
    }

    fn start_sse_run_billing(
        &self,
        run_id: &str,
        _logging_obj: &StreamLoggingObject,
        start_time: DateTime<FixedOffset>,
    ) {
        self.started
            .lock()
            .unwrap()
            .push((run_id.to_owned(), start_time));
    }

    async fn handle_logging_tinyfish_collected_chunks(
        &self,
        call: &TinyfishCollectedChunks<'_>,
    ) -> Result<HandlerOutcome, HandlerError> {
        self.handler_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call.lines.to_vec());
        next_reply(&self.reply)
    }
}

pub(super) fn object_state(object: &StreamLoggingObject) -> IndexMap<String, Json> {
    object
        .read(&mut |state| state.model_call_details.clone())
        .expect("the logging state is present outside dispatch")
}

#[derive(Default)]
pub(super) struct RecordingLogger {
    pub calls: Mutex<Vec<String>>,
}

impl RecordingLogger {
    fn record(&self, hook: &str) {
        self.calls.lock().unwrap().push(hook.to_owned());
    }
}

#[async_trait]
impl CallbackLogger for RecordingLogger {
    fn callback_name(&self) -> &'static str {
        "recording"
    }

    fn log_success_event(&self, _event: &LoggerEvent) -> Result<(), LoggerError> {
        self.record("sync_success");
        Ok(())
    }

    fn log_stream_event(&self, _event: &LoggerEvent) -> Result<(), LoggerError> {
        self.record("sync_stream");
        Ok(())
    }

    async fn async_log_success_event(&self, _event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.record("async_success");
        Ok(())
    }

    async fn async_log_stream_event(&self, _event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.record("async_stream");
        Ok(())
    }
}

pub(super) struct FixedBuilder;

impl StandardPayloadBuilder for FixedBuilder {
    fn build(
        &self,
        _state: &LoggingState,
        _response: &Json,
        _start_time: DateTime<FixedOffset>,
        _end_time: DateTime<FixedOffset>,
    ) -> Option<StandardLoggingPayload> {
        Some(payload())
    }
}

pub(super) struct NoAssembler;

impl StreamAssembler for NoAssembler {
    fn assemble(
        &self,
        _state: &LoggingState,
        _result: &SuccessResult,
        _mode: SuccessMode,
    ) -> Option<Json> {
        None
    }
}

pub(super) struct FixedClock;

impl SuccessClock for FixedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        time("2024-01-01T00:00:02+00:00")
    }
}

pub(super) struct NoFailureCounter;

impl CallbackFailureCounter for NoFailureCounter {
    fn increment_callback_logging_failure(&self, _callback_name: &str) {}
}

pub(super) fn callbacks(
    logger: Arc<RecordingLogger>,
    mut registry: CallbackRegistry,
) -> StreamLoggingCallbacks {
    let sink = CallbackSink::Logger(logger);
    let identity = CallbackIdentity {
        class_name: "recording".to_owned(),
        attributes: IndexMap::new(),
    };
    let registration = registry.register(identity, sink, CallbackList::Success);
    assert_ne!(registration, Registration::SkippedCapacity);
    StreamLoggingCallbacks {
        registry: Arc::new(registry),
        payload_builder: Arc::new(FixedBuilder),
        stream_assembler: Arc::new(NoAssembler),
        clock: Arc::new(FixedClock),
        failure_counters: vec![Arc::new(NoFailureCounter)],
        excluded_fields: Vec::new(),
        known_callback: Arc::new(|_name| false),
        stamp: ResponseStamp {
            id: "chatcmpl-placeholder".to_owned(),
            created: 1_704_067_200,
        },
        privacy: Settings::default(),
    }
}

#[derive(Clone, Default)]
pub(super) struct Captured(pub Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub(super) fn text(&self) -> String {
        let bytes = self.0.lock().unwrap().clone();
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

pub(super) fn capture_logs() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(tracing::Level::ERROR)
        .with_writer(move || writer.clone())
        .finish();
    (captured, set_default(subscriber))
}
