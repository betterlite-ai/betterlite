use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::sections::logging_privacy::Settings;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use tokio::sync::mpsc;

use crate::callback_logger::CallbackLogger;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::exporter_options::failure_counter::CallbackFailureCounter;
use crate::exporter_options::redaction::ResponseStamp;
use crate::fields::payload::StandardLoggingPayload;
use crate::logging_failure::callbacks::FailureCallbacks;
use crate::logging_failure::handlers::async_failure_handler;
use crate::logging_failure::handlers::failure_handler;
use crate::logging_failure::handlers::{SyncFailureJob, run_sync_failure};
use crate::logging_failure::{
    BaselineInvalidationError, BaselineInvalidator, FailureCall, FailureClock, FailureDeps,
    FailurePayloadInput, FailurePayloadSource, FailureRecord, FailureReport, PostCallFlush,
    PostCallFlushError, RateLimitFailureEvent, RateLimitFailureListener, SyncAdmission,
    SyncFailureExecutor,
};
use crate::logging_metadata::exception::ExceptionFacts;
use crate::logging_state::LoggingInit;
use crate::logging_state::LoggingState;
use crate::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use crate::logging_state::types::{CallbackPolicy, DynamicCallbacks, RequestLoggingFlags};
use crate::registry::CallbackRegistry;
use crate::registry::identity::CallbackIdentity;
use crate::registry::phases::CallbackList;
use crate::registry::sink::CallbackSink;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) const PAYLOAD_JSON: &str = r#"{
    "id": "chatcmpl-1",
    "trace_id": "trace-1",
    "session_id": "session-1",
    "call_type": "completion",
    "response_cost": 0.5,
    "status": "failure",
    "status_fields": {"llm_api_status": "failure", "guardrail_status": "not_run"},
    "total_tokens": 5,
    "prompt_tokens": 2,
    "completion_tokens": 3,
    "startTime": 1704067200.0,
    "endTime": 1704067201.0,
    "completionStartTime": 1704067201.0,
    "response_time": 1.0,
    "model_map_information": {},
    "model": "gpt-4o",
    "api_base": "https://api.openai.com",
    "metadata": {},
    "saved_cache_cost": 0.0,
    "request_tags": [],
    "model_parameters": {},
    "hidden_params": {},
    "messages": [{"role": "user", "content": "the secret prompt"}],
    "response": null,
    "error_str": "provider exploded"
}"#;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CallMethod {
    Sync,
    Async,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Flag {
    TurnOffMessages,
    RedactsItself,
    FailsSync,
    FailsAsync,
    FailsSpecial,
}

pub(crate) struct RecordedCall {
    pub(crate) method: CallMethod,
    pub(crate) payload: Arc<StandardLoggingPayload>,
    pub(crate) stream: bool,
    pub(crate) litellm_metadata: Arc<IndexMap<String, Json>>,
}

pub(crate) struct SpecialCall {
    pub(crate) original_model_group: Option<String>,
    pub(crate) exception: Option<String>,
}

pub(crate) struct FakeLogger {
    name: String,
    flags: Vec<Flag>,
    calls: Mutex<Vec<RecordedCall>>,
    special_calls: Mutex<Vec<SpecialCall>>,
}

impl FakeLogger {
    pub(crate) fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            flags: Vec::new(),
            calls: Mutex::new(Vec::new()),
            special_calls: Mutex::new(Vec::new()),
        }
    }

    fn flagged(mut self, flag: Flag) -> Self {
        self.flags.push(flag);
        self
    }

    pub(crate) fn turning_off_messages(self) -> Self {
        self.flagged(Flag::TurnOffMessages)
    }

    pub(crate) fn redacting_messages_itself(self) -> Self {
        self.flagged(Flag::RedactsItself)
    }

    pub(crate) fn failing_sync(self) -> Self {
        self.flagged(Flag::FailsSync)
    }

    pub(crate) fn failing_async(self) -> Self {
        self.flagged(Flag::FailsAsync)
    }

    pub(crate) fn failing_special(self) -> Self {
        self.flagged(Flag::FailsSpecial)
    }

    fn has(&self, flag: Flag) -> bool {
        self.flags.contains(&flag)
    }

    pub(crate) fn calls(&self) -> Vec<&'static str> {
        lock(&self.calls)
            .iter()
            .map(|call| match call.method {
                CallMethod::Sync => "sync",
                CallMethod::Async => "async",
            })
            .collect()
    }

    pub(crate) fn recorded(&self) -> Vec<(CallMethod, Arc<StandardLoggingPayload>, bool)> {
        lock(&self.calls)
            .iter()
            .map(|call| (call.method, Arc::clone(&call.payload), call.stream))
            .collect()
    }

    pub(crate) fn litellm_metadata(&self) -> Vec<(CallMethod, Arc<IndexMap<String, Json>>)> {
        lock(&self.calls)
            .iter()
            .map(|call| (call.method, Arc::clone(&call.litellm_metadata)))
            .collect()
    }

    pub(crate) fn special_calls(&self) -> Vec<(Option<String>, Option<String>)> {
        lock(&self.special_calls)
            .iter()
            .map(|call| (call.original_model_group.clone(), call.exception.clone()))
            .collect()
    }

    fn record(&self, method: CallMethod, event: &LoggerEvent) -> Result<(), LoggerError> {
        lock(&self.calls).push(RecordedCall {
            method,
            payload: Arc::clone(&event.payload),
            stream: event.stream,
            litellm_metadata: Arc::clone(&event.litellm_metadata),
        });
        let failing = match method {
            CallMethod::Sync => self.has(Flag::FailsSync),
            CallMethod::Async => self.has(Flag::FailsAsync),
        };
        if failing {
            return Err(LoggerError::Delivery {
                callback: self.name.clone(),
                message: "the callback refused the event".to_owned(),
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
        self.has(Flag::TurnOffMessages)
    }

    fn redacts_messages_itself(&self) -> bool {
        self.has(Flag::RedactsItself)
    }

    fn log_failure_event(&self, event: &LoggerEvent) -> Result<(), LoggerError> {
        self.record(CallMethod::Sync, event)
    }

    async fn async_log_failure_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.record(CallMethod::Async, &event)
    }
}

#[async_trait]
impl RateLimitFailureListener for FakeLogger {
    fn callback_name(&self) -> &str {
        &self.name
    }

    async fn log_model_group_rate_limit_error(
        &self,
        event: &RateLimitFailureEvent<'_>,
    ) -> Result<(), LoggerError> {
        lock(&self.special_calls).push(SpecialCall {
            original_model_group: event.original_model_group.map(str::to_owned),
            exception: event.exception.message.clone(),
        });
        if self.has(Flag::FailsSpecial) {
            return Err(LoggerError::Delivery {
                callback: self.name.clone(),
                message: "the listener refused the event".to_owned(),
            });
        }
        Ok(())
    }
}

pub(crate) struct FixedClock {
    now: DateTime<FixedOffset>,
}

impl FixedClock {
    pub(crate) fn new(now: &str) -> Self {
        Self { now: moment(now) }
    }
}

impl FailureClock for FixedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        self.now
    }
}

pub(crate) struct FixturePayload {
    payload: Option<StandardLoggingPayload>,
    inputs: Mutex<Vec<String>>,
}

impl FixturePayload {
    pub(crate) fn new(payload: Option<StandardLoggingPayload>) -> Self {
        Self {
            payload,
            inputs: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn calls(&self) -> usize {
        lock(&self.inputs).len()
    }

    pub(crate) fn inputs(&self) -> Vec<String> {
        lock(&self.inputs).clone()
    }
}

impl FailurePayloadSource for FixturePayload {
    fn build(&self, input: FailurePayloadInput<'_>) -> Option<StandardLoggingPayload> {
        lock(&self.inputs).push(format!(
            "{}|{}|{}",
            input.error_str,
            input.start_time.to_rfc3339(),
            input.end_time.to_rfc3339()
        ));
        self.payload.clone()
    }
}

pub(crate) struct RecordingExecutor {
    admission: SyncAdmission,
    run: bool,
    jobs: Mutex<Vec<String>>,
}

impl RecordingExecutor {
    pub(crate) fn running() -> Self {
        Self {
            admission: SyncAdmission::Accepted,
            run: true,
            jobs: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn rejecting() -> Self {
        Self {
            admission: SyncAdmission::Rejected,
            run: false,
            jobs: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn jobs(&self) -> usize {
        lock(&self.jobs).len()
    }
}

impl SyncFailureExecutor for RecordingExecutor {
    fn enqueue_sync_failure(&self, job: SyncFailureJob) -> SyncAdmission {
        lock(&self.jobs).push(job.call_type.clone());
        if self.run && self.admission == SyncAdmission::Accepted {
            run_sync_failure(job);
        }
        self.admission
    }
}

pub(crate) struct CountingFailureCounter {
    count: AtomicUsize,
}

impl CountingFailureCounter {
    pub(crate) fn new() -> Self {
        Self {
            count: AtomicUsize::new(0),
        }
    }

    pub(crate) fn count(&self) -> usize {
        self.count.load(Ordering::SeqCst)
    }
}

impl CallbackFailureCounter for CountingFailureCounter {
    fn increment_callback_logging_failure(&self, _callback_name: &str) {
        self.count.fetch_add(1, Ordering::SeqCst);
    }
}

pub(crate) struct RecordingBaseline {
    pub(crate) reasons: Mutex<Vec<String>>,
}

impl RecordingBaseline {
    pub(crate) fn new() -> Self {
        Self {
            reasons: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl BaselineInvalidator for RecordingBaseline {
    async fn invalidate_baseline(&self, reason: &str) -> Result<(), BaselineInvalidationError> {
        lock(&self.reasons).push(reason.to_owned());
        Ok(())
    }
}

pub(crate) struct RecordingFlush {
    flushed: AtomicUsize,
    fail: bool,
}

impl RecordingFlush {
    pub(crate) fn new() -> Self {
        Self {
            flushed: AtomicUsize::new(0),
            fail: false,
        }
    }

    pub(crate) fn failing() -> Self {
        Self {
            flushed: AtomicUsize::new(0),
            fail: true,
        }
    }

    pub(crate) fn flushed(&self) -> usize {
        self.flushed.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl PostCallFlush for RecordingFlush {
    async fn flush_post_call(&self) -> Result<(), PostCallFlushError> {
        self.flushed.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(PostCallFlushError::Flush);
        }
        Ok(())
    }
}

pub(crate) struct Harness {
    pub(crate) redactor: Redactor,
    pub(crate) payloads: FixturePayload,
    pub(crate) clock: FixedClock,
    pub(crate) executor: RecordingExecutor,
    pub(crate) baseline: RecordingBaseline,
    pub(crate) flush: RecordingFlush,
    pub(crate) counter: CountingFailureCounter,
    pub(crate) listeners: Vec<Arc<dyn RateLimitFailureListener>>,
    pub(crate) privacy: Settings,
}

impl Harness {
    pub(crate) fn new() -> Self {
        Self::with_payload(Some(payload()))
    }

    pub(crate) fn without_payload() -> Self {
        Self::with_payload(None)
    }

    fn with_payload(value: Option<StandardLoggingPayload>) -> Self {
        Self {
            redactor: redactor(),
            payloads: FixturePayload::new(value),
            clock: FixedClock::new("2026-01-01T00:00:02+00:00"),
            executor: RecordingExecutor::running(),
            baseline: RecordingBaseline::new(),
            flush: RecordingFlush::new(),
            counter: CountingFailureCounter::new(),
            listeners: Vec::new(),
            privacy: Settings::default(),
        }
    }

    pub(crate) fn deps(&self) -> FailureDeps<'_> {
        let counter: &(dyn CallbackFailureCounter + Send + Sync) = &self.counter;
        FailureDeps {
            redactor: &self.redactor,
            payloads: &self.payloads,
            clock: &self.clock,
            executor: &self.executor,
            baseline: Some(&self.baseline),
            flush: Some(&self.flush),
            special_listeners: &self.listeners,
            counters: vec![counter],
            excluded_fields: &[],
            stamp: stamp(),
            privacy: self.privacy,
        }
    }

    pub(crate) fn listen(&mut self, logger: &Arc<FakeLogger>) {
        self.listeners.push(Arc::<FakeLogger>::clone(logger));
    }
}

pub(crate) fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor compiles")
}

pub(crate) fn moment(value: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(value).expect("the fixture is RFC 3339")
}

pub(crate) fn object(pairs: &[(&str, Json)]) -> Json {
    let mut values = IndexMap::new();
    for (name, value) in pairs {
        values.insert((*name).to_owned(), value.clone());
    }
    Json::Object(values)
}

pub(crate) fn params(pairs: &[(&str, Json)]) -> IndexMap<String, Json> {
    let mut values = IndexMap::new();
    for (name, value) in pairs {
        values.insert((*name).to_owned(), value.clone());
    }
    values
}

pub(crate) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(crate) fn payload() -> StandardLoggingPayload {
    serde_json::from_str(PAYLOAD_JSON).expect("the payload fixture parses")
}

pub(crate) fn exception(message: &str) -> ExceptionFacts {
    let text = serde_json::to_string(message).expect("the message encodes");
    let encoded = format!(r#"{{"class_name": "APIError", "message": {text}}}"#);
    serde_json::from_str(&encoded).expect("the exception fixture parses")
}

pub(crate) fn report(message: &str) -> FailureReport {
    FailureReport {
        exception: Arc::new(exception(message)),
        traceback: "Traceback (most recent call last): sk-abcdefghijklmnop".to_owned(),
        start_time: None,
        end_time: None,
    }
}

pub(crate) fn stamp() -> ResponseStamp {
    ResponseStamp {
        id: "chatcmpl-redacted".to_owned(),
        created: 1_704_067_200,
    }
}

pub(crate) fn init() -> LoggingInit {
    LoggingInit {
        model: "gpt-4o".to_owned(),
        call_type: "completion".to_owned(),
        start_time: moment("2026-01-01T00:00:00+00:00"),
        litellm_call_id: "call-1".to_owned(),
        function_id: "fn-1".to_owned(),
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
    }
}

pub(crate) fn build(init: LoggingInit) -> LoggingState {
    let mut context = CorrelationContext::default();
    build_in(init, &mut context, CorrelationSettings::default())
}

pub(crate) fn build_in(
    init: LoggingInit,
    context: &mut CorrelationContext,
    settings: CorrelationSettings,
) -> LoggingState {
    LoggingState::new(
        init,
        CorrelationSetup { context, settings },
        &redactor(),
        || "generated-trace".to_owned(),
        || Duration::ZERO,
    )
    .expect("the fixture builds")
}

fn with_sink(loggers: &[Arc<FakeLogger>], extra: Option<CallbackSink>) -> CallbackRegistry {
    let mut registry = CallbackRegistry::new(100);
    for logger in loggers {
        let _registration = registry.register(
            CallbackIdentity {
                class_name: logger.name.clone(),
                attributes: IndexMap::new(),
            },
            CallbackSink::Logger(Arc::<FakeLogger>::clone(logger)),
            CallbackList::Failure,
        );
    }
    if let Some(sink) = extra {
        let _registration = registry.register(
            CallbackIdentity {
                class_name: "legacy".to_owned(),
                attributes: IndexMap::new(),
            },
            sink,
            CallbackList::Failure,
        );
    }
    registry
}

pub(crate) fn registry(loggers: &[Arc<FakeLogger>]) -> CallbackRegistry {
    with_sink(loggers, None)
}

pub(crate) fn registry_with_channel(loggers: &[Arc<FakeLogger>]) -> CallbackRegistry {
    let (sender, _receiver) = mpsc::channel(4);
    with_sink(loggers, Some(CallbackSink::Channel(sender)))
}

pub(crate) fn callbacks(registry: &CallbackRegistry) -> FailureCallbacks<'_> {
    FailureCallbacks {
        registry,
        dynamic: None,
    }
}

pub(crate) fn call<'state>(
    state: &'state mut LoggingState,
    record: &'state mut FailureRecord,
    context: &'state mut CorrelationContext,
) -> FailureCall<'state> {
    FailureCall {
        state,
        record,
        context,
    }
}

pub(crate) fn run_sync_handler(
    state: &mut LoggingState,
    record: &mut FailureRecord,
    context: &mut CorrelationContext,
    registry: &CallbackRegistry,
    harness: &Harness,
) {
    failure_handler(
        &mut call(state, record, context),
        &report("provider exploded"),
        &callbacks(registry),
        &harness.deps(),
    );
}

pub(crate) async fn run_async_handler(
    state: &mut LoggingState,
    record: &mut FailureRecord,
    context: &mut CorrelationContext,
    registry: &CallbackRegistry,
    harness: &Harness,
) {
    let mut failure_call = call(state, record, context);
    let callbacks = callbacks(registry);
    let deps = harness.deps();
    async_failure_handler(
        &mut failure_call,
        &report("provider exploded"),
        &callbacks,
        &deps,
    )
    .await;
}
