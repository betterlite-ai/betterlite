use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::sections::logging_privacy::Settings;
use betterlite_logging::callback_logger::CallbackLogger;
use betterlite_logging::callback_logger::error::LoggerError;
use betterlite_logging::callback_logger::event::LoggerEvent;
use betterlite_logging::exporter_options::failure_counter::CallbackFailureCounter;
use betterlite_logging::exporter_options::redaction::ResponseStamp;
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_logging::logging_success::ports::{
    StandardPayloadBuilder, StreamAssembler, SuccessClock,
};
use betterlite_logging::logging_success::types::{SuccessMode, SuccessRequest, SuccessResult};
use betterlite_logging::registry::identity::CallbackIdentity;
use betterlite_logging::registry::phases::CallbackList;
use betterlite_logging::registry::sink::CallbackSink;
use betterlite_logging::registry::{CallbackRegistry, Registration};
use betterlite_logging::worker::WorkerSettings;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;

use crate::caching::hit_callbacks::context::{
    CACHING_DETAILS, CacheHitContext, record_cache_hit_context,
};
use crate::caching::hit_callbacks::job::{CacheHitCallbacks, OwnedSuccessHandler};

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

pub(super) fn time(value: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(value).unwrap()
}

pub(super) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(super) fn object(fields: &[(&str, Json)]) -> Json {
    let mut object = IndexMap::new();
    for (name, value) in fields {
        object.insert((*name).to_owned(), value.clone());
    }
    Json::Object(object)
}

pub(super) fn params(fields: &[(&str, Json)]) -> IndexMap<String, Json> {
    let mut map = IndexMap::new();
    for (name, value) in fields {
        map.insert((*name).to_owned(), value.clone());
    }
    map
}

pub(super) fn encoded(value: &Json) -> String {
    serde_json::to_string(value).unwrap()
}

pub(super) fn cached() -> Json {
    object(&[
        ("id", text("chatcmpl-1")),
        ("object", text("chat.completion")),
        ("choices", Json::List(Vec::new())),
    ])
}

pub(super) fn logging_state(stream: Option<bool>) -> LoggingState {
    let mut context = CorrelationContext::default();
    LoggingState::new(
        LoggingInit {
            model: "gpt-4o".to_owned(),
            call_type: "completion".to_owned(),
            start_time: time("2024-01-01T00:00:00+00:00"),
            litellm_call_id: "call-1".to_owned(),
            function_id: "function-1".to_owned(),
            messages: None,
            stream,
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
        },
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        &Redactor::new(16).unwrap(),
        || "generated-trace".to_owned(),
        || Duration::ZERO,
    )
    .unwrap()
}

pub(super) fn base_hit<'hit>(
    kwargs: &'hit IndexMap<String, Json>,
    cached_result: &'hit Json,
) -> CacheHitContext<'hit> {
    CacheHitContext {
        model: "gpt-4o",
        custom_llm_provider: Some("openai"),
        preset_cache_key: Some("preset-key"),
        cache_duration_ms: Some(12.5),
        kwargs,
        cached_result,
        is_async: true,
        is_embedding: false,
    }
}

pub(super) fn details(state: &LoggingState, keys: &[&str]) -> String {
    let mut values = IndexMap::new();
    for key in keys {
        let value = state
            .model_call_details
            .get(*key)
            .cloned()
            .unwrap_or_else(|| Json::Text("<missing>".to_owned()));
        values.insert((*key).to_owned(), value);
    }
    encoded(&Json::Object(values))
}

pub(super) fn worker_settings(max_queue_size: usize, concurrency: usize) -> WorkerSettings {
    WorkerSettings {
        max_queue_size,
        concurrency,
        event_timeout: Duration::from_secs(5),
        shutdown_drain: Duration::from_secs(5),
        timeout_summary_window: Duration::from_secs(1),
    }
}

#[derive(Default)]
pub(super) struct Calls(Mutex<Vec<&'static str>>);

impl Calls {
    pub(super) fn record(&self, event: &'static str) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }

    pub(super) fn events(&self) -> Vec<&'static str> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

pub(super) struct FakeLogger {
    pub(super) name: String,
    pub(super) calls: Arc<Calls>,
    pub(super) fail: bool,
}

#[async_trait]
impl CallbackLogger for FakeLogger {
    fn callback_name(&self) -> &str {
        &self.name
    }

    async fn async_log_success_event(&self, _event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.calls.record("async");
        if self.fail {
            return Err(LoggerError::Delivery {
                callback: self.name.clone(),
                message: "the transport is down".to_owned(),
            });
        }
        Ok(())
    }
}

pub(super) struct FakeBuilder {
    pub(super) payload: Option<StandardLoggingPayload>,
    pub(super) calls: Arc<Calls>,
}

impl StandardPayloadBuilder for FakeBuilder {
    fn build(
        &self,
        state: &LoggingState,
        _response: &Json,
        _start_time: DateTime<FixedOffset>,
        _end_time: DateTime<FixedOffset>,
    ) -> Option<StandardLoggingPayload> {
        let recorded = state.model_call_details.contains_key(CACHING_DETAILS);
        let marked = state.model_call_details.get("cache_hit") == Some(&Json::Bool(true));
        if state
            .model_call_details
            .contains_key("has_logged_sync_success")
        {
            self.calls.record("build-after-sync");
        } else if recorded && marked {
            self.calls.record("build-with-cache-hit");
        } else {
            self.calls.record("build-without-cache-hit");
        }
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
    pub(super) now: DateTime<FixedOffset>,
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
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl CallbackFailureCounter for RecordingCounter {
    fn increment_callback_logging_failure(&self, callback_name: &str) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(callback_name.to_owned());
    }
}

pub(super) struct Fixture {
    pub(super) calls: Arc<Calls>,
    pub(super) counter: Arc<RecordingCounter>,
    pub(super) fail: bool,
}

impl Fixture {
    pub(super) fn new(fail: bool) -> Self {
        Self {
            calls: Arc::new(Calls::default()),
            counter: Arc::new(RecordingCounter::default()),
            fail,
        }
    }

    pub(super) fn owned_handler(&self) -> OwnedSuccessHandler {
        let logger = Arc::new(FakeLogger {
            name: "langfuse".to_owned(),
            calls: Arc::clone(&self.calls),
            fail: self.fail,
        });
        let mut registry = CallbackRegistry::new(10);
        let identity = CallbackIdentity {
            class_name: logger.name.clone(),
            attributes: IndexMap::new(),
        };
        let handle: Arc<dyn CallbackLogger> = logger;
        let sink = CallbackSink::Logger(handle);
        let registration = registry.register(identity, sink, CallbackList::Callbacks);
        assert_ne!(registration, Registration::SkippedCapacity);
        let counter = Arc::clone(&self.counter);
        let counter_handle: Arc<dyn CallbackFailureCounter + Send + Sync> = counter;
        OwnedSuccessHandler {
            registry: Arc::new(registry),
            payload_builder: Arc::new(FakeBuilder {
                payload: Some(serde_json::from_str(PAYLOAD).unwrap()),
                calls: Arc::clone(&self.calls),
            }),
            stream_assembler: Arc::new(FakeAssembler),
            clock: Arc::new(FixedClock {
                now: time("2024-01-01T00:00:02+00:00"),
            }),
            failure_counters: vec![counter_handle],
            excluded_fields: Vec::new(),
            known_callback: Arc::new(|_name: &str| false),
            stamp: ResponseStamp {
                id: "chatcmpl-1".to_owned(),
                created: 1_704_067_200,
            },
            privacy: Settings::default(),
        }
    }

    pub(super) fn callbacks(&self, result: &Json) -> CacheHitCallbacks {
        CacheHitCallbacks {
            handler: self.owned_handler(),
            state: logging_state(None),
            correlation: CorrelationContext::default(),
            result: SuccessResult::Chat(result.clone()),
            start_time: Some(time("2024-01-01T00:00:00+00:00")),
            end_time: Some(time("2024-01-01T00:00:02+00:00")),
        }
    }
}

pub(super) fn hit_kwargs() -> IndexMap<String, Json> {
    params(&[
        (
            "messages",
            Json::List(vec![object(&[
                ("role", text("user")),
                ("content", text("hi")),
            ])]),
        ),
        ("stream", Json::Bool(false)),
    ])
}

pub(super) fn recorded_state(result: &Json) -> LoggingState {
    let kwargs = hit_kwargs();
    let mut state = logging_state(None);
    record_cache_hit_context(&mut state, &base_hit(&kwargs, result))
        .expect("the cache hit is recorded");
    state
}

pub(super) fn success_request(result: SuccessResult) -> SuccessRequest {
    SuccessRequest {
        result,
        start_time: Some(time("2024-01-01T00:00:00+00:00")),
        end_time: Some(time("2024-01-01T00:00:02+00:00")),
        cache_hit: Some(true),
        prefer_async_handlers: false,
        standard_logging_object: None,
        guardrail_snapshot: None,
    }
}
