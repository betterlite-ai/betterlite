use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_context::agentic_callback::{
    AgenticCall, AgenticCallback, AgenticFlag, AgenticGate, AgenticHookError, AgenticLogging,
    AgenticLoopCall, AgenticOverrides,
};
use betterlite_context::agentic_loop_plan::{
    AgenticLoopPlan, AgenticLoopRequestPatch, FollowupMessage, JsonMap,
};
use betterlite_openai::stream::chunk::Chunk;
use futures_util::StreamExt as _;
use futures_util::stream::{self, BoxStream};
use serde_json::value::RawValue;
use tracing::Event;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, Subscriber, set_default};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use super::super::error::{AgenticLoopError, ChatCallError};
use super::super::followup::FollowupCall;
use super::super::settings::AgenticSettings;
use super::super::{ChatContinuation, ChatLoopCall, ChatLoopResult, FakeStreamPort, LoggingHandle};

pub(crate) fn raw(text: &str) -> Box<RawValue> {
    RawValue::from_string(text.to_owned()).unwrap()
}

pub(crate) fn json_map(entries: &[(&str, &str)]) -> JsonMap {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), raw(value)))
        .collect()
}

pub(crate) fn raw_text(map: &JsonMap, key: &str) -> Option<String> {
    map.get(key).map(|value| value.get().to_owned())
}

pub(crate) fn messages() -> Vec<FollowupMessage> {
    vec![FollowupMessage(raw(
        r#"{"role":"user","content":"what is the weather"}"#,
    ))]
}

pub(crate) fn tool_messages() -> Vec<FollowupMessage> {
    vec![FollowupMessage(raw(
        r#"{"type":"function","function":{"name":"get_weather"}}"#,
    ))]
}

pub(crate) fn empty_stream() -> BoxStream<'static, Chunk> {
    stream::iter(Vec::new()).boxed()
}

pub(crate) fn hook_failure(message: &str) -> AgenticHookError {
    AgenticHookError::Failed {
        callback: "test".to_owned(),
        message: message.to_owned(),
    }
}

pub(crate) fn overrides(
    should_run: AgenticFlag,
    build_plan: AgenticFlag,
    post_response: AgenticFlag,
) -> AgenticOverrides {
    AgenticOverrides {
        should_run,
        run_loop: false,
        build_plan,
        post_response,
        cleanup: false,
    }
}

pub(crate) fn gate(run: bool, tools: JsonMap) -> AgenticGate {
    AgenticGate { run, tools }
}

pub(crate) fn patch(messages: Option<Vec<FollowupMessage>>) -> AgenticLoopRequestPatch {
    AgenticLoopRequestPatch {
        model: None,
        messages,
        tools: None,
        max_tokens: None,
        optional_params: JsonMap::new(),
        kwargs: JsonMap::new(),
    }
}

pub(crate) fn plan(
    request_patch: Option<AgenticLoopRequestPatch>,
    response_override: Option<Box<RawValue>>,
    terminate: AgenticFlag,
    run_agentic_loop: AgenticFlag,
) -> AgenticLoopPlan {
    AgenticLoopPlan {
        run_agentic_loop,
        request_patch,
        response_override,
        terminate,
        stop_reason: None,
        metadata: JsonMap::new(),
    }
}

pub(crate) fn metadata_of(kwargs: &JsonMap) -> JsonMap {
    let raw = kwargs.get("litellm_metadata").expect("litellm_metadata");
    serde_json::from_str(raw.get()).expect("the metadata map")
}

pub(crate) fn response_text(result: ChatLoopResult) -> String {
    match result {
        ChatLoopResult::Response(raw) => raw.get().to_owned(),
        ChatLoopResult::Stream(_) => panic!("expected a response result"),
    }
}

pub(crate) fn optional_text(result: Option<ChatLoopResult>) -> String {
    match result {
        Some(result) => response_text(result),
        None => panic!("expected a loop result"),
    }
}

pub(crate) fn loop_error<T>(result: Result<T, AgenticLoopError>) -> AgenticLoopError {
    match result {
        Ok(_) => panic!("expected the loop to fail"),
        Err(error) => error,
    }
}

pub(crate) fn call_error<T>(
    result: Result<T, ChatCallError<StopError>>,
) -> ChatCallError<StopError> {
    match result {
        Ok(_) => panic!("expected the call to fail"),
        Err(error) => error,
    }
}

pub(crate) fn logging() -> AgenticLogging {
    AgenticLogging {
        call_id: Some("call-1".to_owned()),
        agentic_loop_params: None,
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub(crate) struct StopError {
    pub message: &'static str,
}

#[derive(Clone)]
pub(crate) struct RecordedCall {
    pub model: String,
    pub messages: Vec<FollowupMessage>,
    pub params: JsonMap,
    pub kwargs: JsonMap,
}

pub(crate) fn first_call(continuation: &FakeContinuation) -> RecordedCall {
    let mut calls = continuation.calls();
    calls.remove(0)
}

pub(crate) struct FakeContinuation {
    script: Mutex<Option<Result<ChatLoopResult, StopError>>>,
    calls: Mutex<Vec<RecordedCall>>,
}

impl FakeContinuation {
    pub(crate) fn responding(result: ChatLoopResult) -> Self {
        Self {
            script: Mutex::new(Some(Ok(result))),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn failing(message: &'static str) -> Self {
        Self {
            script: Mutex::new(Some(Err(StopError { message }))),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn calls(&self) -> Vec<RecordedCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn call_count(&self) -> usize {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

#[async_trait]
impl ChatContinuation for FakeContinuation {
    type Error = StopError;

    async fn complete(
        &self,
        model: &str,
        messages: &[FollowupMessage],
        params: &JsonMap,
        kwargs: &JsonMap,
    ) -> Result<ChatLoopResult, ChatCallError<Self::Error>> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedCall {
                model: model.to_owned(),
                messages: messages.to_vec(),
                params: params.clone(),
                kwargs: kwargs.clone(),
            });
        let scripted = self
            .script
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        match scripted {
            Some(Ok(result)) => Ok(result),
            Some(Err(error)) => Err(ChatCallError::Completion(error)),
            None => Err(ChatCallError::Completion(StopError {
                message: "the continuation was called twice",
            })),
        }
    }
}

#[derive(Clone)]
pub(crate) struct WrapRecord {
    pub response: String,
    pub model: String,
    pub provider: String,
    pub has_logging: bool,
}

#[derive(Default)]
pub(crate) struct RecordingPort {
    records: Mutex<Vec<WrapRecord>>,
}

impl RecordingPort {
    pub(crate) fn records(&self) -> Vec<WrapRecord> {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl FakeStreamPort for RecordingPort {
    fn wrap_model_response(
        &self,
        response: Box<RawValue>,
        model: &str,
        provider: &str,
        logging: &LoggingHandle<'_>,
    ) -> ChatLoopResult {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(WrapRecord {
                response: response.get().to_owned(),
                model: model.to_owned(),
                provider: provider.to_owned(),
                has_logging: matches!(logging, LoggingHandle::Object(_)),
            });
        ChatLoopResult::Response(raw(r#"{"fake_stream":true}"#))
    }
}

#[derive(Clone)]
pub(crate) struct GateRecord {
    pub model: String,
    pub stream: bool,
    pub provider: String,
    pub kwargs: JsonMap,
    pub tools: Option<Vec<FollowupMessage>>,
}

#[derive(Clone)]
pub(crate) struct LoopRecord {
    pub tools: JsonMap,
    pub model: String,
    pub messages: Vec<String>,
    pub response: String,
    pub log_call_id: Option<String>,
    pub stream: bool,
    pub optional_params: JsonMap,
    pub kwargs: JsonMap,
}

#[derive(Clone)]
pub(crate) struct PostRecord {
    pub kwargs: JsonMap,
    pub stop_reason: Option<String>,
}

#[derive(Clone)]
pub(crate) struct CleanupRecord {
    pub stop_reason: Option<String>,
    pub kwargs: JsonMap,
}

#[derive(Clone, Default)]
pub(crate) struct CallbackRecords {
    pub gate_calls: Vec<GateRecord>,
    pub legacy_calls: Vec<LoopRecord>,
    pub plan_calls: Vec<LoopRecord>,
    pub post_calls: Vec<PostRecord>,
    pub cleanup_calls: Vec<CleanupRecord>,
}

#[derive(Default)]
pub(crate) struct Script {
    pub overrides: AgenticOverrides,
    pub gate: Option<Result<AgenticGate, AgenticHookError>>,
    pub legacy: Option<Result<Option<Box<RawValue>>, AgenticHookError>>,
    pub plan: Option<Result<AgenticLoopPlan, AgenticHookError>>,
    pub post: Option<Result<Box<RawValue>, AgenticHookError>>,
    pub cleanup: Option<Result<(), AgenticHookError>>,
}

impl Script {
    pub(crate) fn with_overrides(mut self, overrides: AgenticOverrides) -> Self {
        self.overrides = overrides;
        self
    }

    pub(crate) fn with_gate(mut self, gate: AgenticGate) -> Self {
        self.gate = Some(Ok(gate));
        self
    }

    pub(crate) fn with_gate_error(mut self, message: &str) -> Self {
        self.gate = Some(Err(hook_failure(message)));
        self
    }

    pub(crate) fn with_legacy(mut self, response: Option<Box<RawValue>>) -> Self {
        self.legacy = Some(Ok(response));
        self
    }

    pub(crate) fn with_legacy_error(mut self, message: &str) -> Self {
        self.legacy = Some(Err(hook_failure(message)));
        self
    }

    pub(crate) fn with_plan(mut self, plan: AgenticLoopPlan) -> Self {
        self.plan = Some(Ok(plan));
        self
    }

    pub(crate) fn with_plan_error(mut self, message: &str) -> Self {
        self.plan = Some(Err(hook_failure(message)));
        self
    }

    pub(crate) fn with_post(mut self, response: Box<RawValue>) -> Self {
        self.post = Some(Ok(response));
        self
    }

    pub(crate) fn with_post_error(mut self, message: &str) -> Self {
        self.post = Some(Err(hook_failure(message)));
        self
    }

    pub(crate) fn with_cleanup_error(mut self, message: &str) -> Self {
        self.cleanup = Some(Err(hook_failure(message)));
        self
    }
}

pub(crate) struct ScriptedCallback {
    script: Mutex<Script>,
    records: Mutex<CallbackRecords>,
}

impl ScriptedCallback {
    pub(crate) fn new(script: Script) -> Self {
        Self {
            script: Mutex::new(script),
            records: Mutex::new(CallbackRecords::default()),
        }
    }

    pub(crate) fn records(&self) -> CallbackRecords {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record_loop(&self, call: &AgenticLoopCall<'_>, legacy: bool) {
        let record = LoopRecord {
            tools: call.tools.clone(),
            model: call.model.to_owned(),
            messages: call
                .messages
                .iter()
                .map(|message| message.0.get().to_owned())
                .collect(),
            response: call.response.get().to_owned(),
            log_call_id: call.logging.and_then(|logging| logging.call_id.clone()),
            stream: call.stream,
            optional_params: call.optional_params.clone(),
            kwargs: call.kwargs.clone(),
        };
        let mut records = self.records.lock().unwrap_or_else(PoisonError::into_inner);
        if legacy {
            records.legacy_calls.push(record);
        } else {
            records.plan_calls.push(record);
        }
    }
}

#[async_trait]
impl AgenticCallback for ScriptedCallback {
    fn callback_name(&self) -> &'static str {
        "scripted"
    }

    fn overrides(&self) -> AgenticOverrides {
        self.script
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .overrides
    }

    async fn should_run_agentic_loop(
        &self,
        call: &AgenticCall<'_>,
    ) -> Result<AgenticGate, AgenticHookError> {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .gate_calls
            .push(GateRecord {
                model: call.model.to_owned(),
                stream: call.stream,
                provider: call.custom_llm_provider.to_owned(),
                kwargs: call.kwargs.clone(),
                tools: call.tools.map(|tools| tools.to_vec()),
            });
        let scripted = self
            .script
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .gate
            .take();
        match scripted {
            Some(gate) => gate,
            None => Ok(AgenticGate::default()),
        }
    }

    async fn build_agentic_loop_plan(
        &self,
        call: &AgenticLoopCall<'_>,
    ) -> Result<AgenticLoopPlan, AgenticHookError> {
        self.record_loop(call, false);
        let scripted = self
            .script
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .plan
            .take();
        match scripted {
            Some(plan) => plan,
            None => Ok(AgenticLoopPlan::default()),
        }
    }

    async fn run_agentic_loop(
        &self,
        call: &AgenticLoopCall<'_>,
    ) -> Result<Option<Box<RawValue>>, AgenticHookError> {
        self.record_loop(call, true);
        let scripted = self
            .script
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .legacy
            .take();
        match scripted {
            Some(legacy) => legacy,
            None => Ok(None),
        }
    }

    async fn post_agentic_loop_response(
        &self,
        response: Box<RawValue>,
        plan: &AgenticLoopPlan,
        kwargs: &JsonMap,
    ) -> Result<Box<RawValue>, AgenticHookError> {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .post_calls
            .push(PostRecord {
                kwargs: kwargs.clone(),
                stop_reason: plan.stop_reason.clone(),
            });
        let scripted = self
            .script
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .post
            .take();
        match scripted {
            Some(hooked) => hooked,
            None => Ok(response),
        }
    }

    async fn agentic_loop_cleanup(
        &self,
        plan: &AgenticLoopPlan,
        kwargs: &JsonMap,
    ) -> Result<(), AgenticHookError> {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .cleanup_calls
            .push(CleanupRecord {
                stop_reason: plan.stop_reason.clone(),
                kwargs: kwargs.clone(),
            });
        let scripted = self
            .script
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .cleanup
            .take();
        match scripted {
            Some(result) => result,
            None => Ok(()),
        }
    }
}

pub(crate) fn callbacks(items: &[&Arc<ScriptedCallback>]) -> Vec<Arc<dyn AgenticCallback>> {
    let mut collected: Vec<Arc<dyn AgenticCallback>> = Vec::with_capacity(items.len());
    for item in items {
        collected.push(Arc::clone(*item));
    }
    collected
}

pub(crate) struct Fixture {
    pub response: Box<RawValue>,
    pub messages: Vec<FollowupMessage>,
    pub optional_params: JsonMap,
    pub kwargs: JsonMap,
    pub logging: AgenticLogging,
    pub model: &'static str,
    pub provider: &'static str,
    pub stream: bool,
}

pub(crate) fn fixture() -> Fixture {
    Fixture {
        response: raw(
            r#"{"id":"chatcmpl-1","object":"chat.completion","created":1,"model":"gpt-4o","choices":[]}"#,
        ),
        messages: messages(),
        optional_params: JsonMap::new(),
        kwargs: JsonMap::new(),
        logging: logging(),
        model: "gpt-4o",
        provider: "openai",
        stream: false,
    }
}

impl Fixture {
    pub(crate) fn call(&self) -> ChatLoopCall<'_> {
        self.call_with(LoggingHandle::Object(&self.logging))
    }

    pub(crate) fn call_with<'fixture>(
        &'fixture self,
        logging: LoggingHandle<'fixture>,
    ) -> ChatLoopCall<'fixture> {
        ChatLoopCall {
            response: &self.response,
            model: self.model,
            messages: &self.messages,
            optional_params: &self.optional_params,
            kwargs: &self.kwargs,
            logging,
            custom_llm_provider: self.provider,
            stream: self.stream,
        }
    }
}

pub(crate) struct FollowupParts {
    pub plan: AgenticLoopPlan,
    pub optional_params: JsonMap,
    pub kwargs: JsonMap,
    pub logging: AgenticLogging,
    pub settings: AgenticSettings,
    pub callback: Arc<ScriptedCallback>,
    pub continuation: FakeContinuation,
    pub port: RecordingPort,
    pub fingerprint: &'static str,
    pub model: &'static str,
    pub provider: &'static str,
}

pub(crate) fn followup_parts() -> FollowupParts {
    FollowupParts {
        plan: plan(Some(patch(Some(messages()))), None, false, true),
        optional_params: JsonMap::new(),
        kwargs: JsonMap::new(),
        logging: logging(),
        settings: AgenticSettings {
            depth: 0,
            max_loops: 3,
            fingerprints: Vec::new(),
        },
        callback: Arc::new(ScriptedCallback::new(Script::default())),
        continuation: FakeContinuation::responding(ChatLoopResult::Response(raw(
            r#"{"followup":true}"#,
        ))),
        port: RecordingPort::default(),
        fingerprint: "fingerprint-1",
        model: "gpt-4o",
        provider: "openai",
    }
}

impl FollowupParts {
    pub(crate) fn followup(&self) -> FollowupCall<'_, FakeContinuation, RecordingPort> {
        self.followup_with(LoggingHandle::Object(&self.logging))
    }

    pub(crate) fn followup_with<'parts>(
        &'parts self,
        logging: LoggingHandle<'parts>,
    ) -> FollowupCall<'parts, FakeContinuation, RecordingPort> {
        FollowupCall {
            plan: &self.plan,
            callback: self.callback.as_ref(),
            model: self.model,
            optional_params: &self.optional_params,
            kwargs: &self.kwargs,
            logging,
            custom_llm_provider: self.provider,
            fingerprint: self.fingerprint,
            settings: &self.settings,
            continuation: &self.continuation,
            fake_stream: &self.port,
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct Capture(Arc<Mutex<Vec<String>>>);

impl Capture {
    pub(crate) fn messages(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Default)]
struct MessageVisitor(String);

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            value.clone_into(&mut self.0);
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Capture {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(visitor.0);
    }
}

pub(crate) fn capture() -> (Capture, DefaultGuard) {
    let captured = Capture::default();
    let guard = set_default(registry().with(captured.clone()));
    (captured, guard)
}
