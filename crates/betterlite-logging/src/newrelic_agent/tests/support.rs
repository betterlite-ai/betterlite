use std::collections::BTreeMap;
use std::io::{Result as IoResult, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;
use uuid::Uuid;

use crate::callback_logger::event::LoggerEvent;
use crate::exporter_options::failure_counter::CallbackFailureCounter;
use crate::fields::payload::StandardLoggingPayload;
use crate::fields::timestamps::Moment;
use crate::newrelic_agent::NewRelicAgentError;
use crate::newrelic_agent::agent::{NewRelicAgent, NewRelicApplication};
use crate::newrelic_agent::backoff::MetricBackoff;
use crate::newrelic_agent::events::SUPPORTABILITY_METRIC_PREFIX;
use crate::newrelic_agent::init::{NewRelicInit, NewRelicInitParams};
use crate::newrelic_agent::logger::{NewRelicDeps, NewRelicLogger};
use crate::newrelic_extraction::ids::UuidSource;
use crate::newrelic_extraction::sources::CallKwargs;
use crate::newrelic_metrics::clock::Clock;

pub(super) const CLOCK_START: u64 = 1_700_000_000;

const STANDARD_PAYLOAD: &str = r#"{
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
    "model_id": "model-1",
    "model_group": "production",
    "custom_llm_provider": "openai",
    "api_base": "https://api.openai.com",
    "metadata": {},
    "cache_hit": false,
    "saved_cache_cost": 0.0,
    "request_tags": [],
    "messages": [{"role": "user", "content": "hello"}],
    "response": {
        "id": "chatcmpl-1",
        "model": "gpt-4o-mini",
        "choices": [{"finish_reason": "stop", "message": {"role": "assistant", "content": "hi"}}],
        "usage": {"prompt_tokens": 2, "completion_tokens": 3, "total_tokens": 5}
    },
    "model_parameters": {"temperature": 0.5, "max_tokens": 100},
    "hidden_params": {}
}"#;

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

pub(super) fn object(text: &str) -> IndexMap<String, Json> {
    match json(text) {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("expected a JSON object: {text}")
        }
    }
}

pub(super) fn payload() -> StandardLoggingPayload {
    serde_json::from_str(STANDARD_PAYLOAD).unwrap()
}

pub(super) fn event(payload: StandardLoggingPayload) -> Arc<LoggerEvent> {
    event_with_dynamic(payload, Json::Object(IndexMap::new()))
}

pub(super) fn event_with_dynamic(
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

pub(super) fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn attribute<'event>(event: &'event RecordedEvent, name: &str) -> Option<&'event Json> {
    event.attributes.get(name)
}

pub(super) fn number_value(attributes: &IndexMap<String, Json>, name: &str) -> f64 {
    match attributes.get(name) {
        Some(Json::Number(number)) => number.as_f64().unwrap(),
        _ => panic!("missing number attribute {name}"),
    }
}

pub(super) struct RecordedEvent {
    pub name: String,
    pub attributes: IndexMap<String, Json>,
}

#[derive(Default)]
pub(super) struct FakeApplication {
    enabled: AtomicBool,
    failing_events: Mutex<Vec<String>>,
    fail_metrics: AtomicUsize,
    events: Mutex<Vec<RecordedEvent>>,
    metrics: Mutex<Vec<(String, i64)>>,
}

impl FakeApplication {
    pub(super) fn new(enabled: bool) -> Arc<Self> {
        Arc::new(Self {
            enabled: AtomicBool::new(enabled),
            failing_events: Mutex::new(Vec::new()),
            fail_metrics: AtomicUsize::new(0),
            events: Mutex::new(Vec::new()),
            metrics: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::SeqCst);
    }

    pub(super) fn fail_event(&self, name: &str) {
        self.failing_events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(name.to_owned());
    }

    pub(super) fn fail_next_metrics(&self, count: usize) {
        self.fail_metrics.store(count, Ordering::SeqCst);
    }

    fn take_event_failure(&self, name: &str) -> bool {
        let mut failing = self
            .failing_events
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let Some(index) = failing.iter().position(|candidate| candidate == name) else {
            return false;
        };
        failing.remove(index);
        true
    }

    pub(super) fn events(&self) -> Vec<RecordedEvent> {
        let events = self.events.lock().unwrap_or_else(PoisonError::into_inner);
        events
            .iter()
            .map(|event| RecordedEvent {
                name: event.name.clone(),
                attributes: event.attributes.clone(),
            })
            .collect()
    }

    pub(super) fn events_named(&self, name: &str) -> Vec<RecordedEvent> {
        self.events()
            .into_iter()
            .filter(|event| event.name == name)
            .collect()
    }

    pub(super) fn metrics(&self) -> Vec<(String, i64)> {
        self.metrics
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn metrics_named(&self, name: &str) -> Vec<i64> {
        self.metrics()
            .into_iter()
            .filter(|(metric, _value)| metric == name)
            .map(|(_metric, value)| value)
            .collect()
    }

    pub(super) fn supportability_metrics(&self) -> Vec<String> {
        self.metrics()
            .into_iter()
            .map(|(metric, _value)| metric)
            .filter(|metric| metric.starts_with(SUPPORTABILITY_METRIC_PREFIX))
            .collect()
    }
}

impl NewRelicApplication for FakeApplication {
    fn enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    fn record_custom_event(
        &self,
        name: &str,
        attributes: &IndexMap<String, Json>,
    ) -> Result<(), NewRelicAgentError> {
        if self.take_event_failure(name) {
            return Err(NewRelicAgentError::Event {
                message: "event boom".to_owned(),
            });
        }
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedEvent {
                name: name.to_owned(),
                attributes: attributes.clone(),
            });
        Ok(())
    }

    fn record_custom_metric(&self, name: &str, value: i64) -> Result<(), NewRelicAgentError> {
        if self.fail_metrics.load(Ordering::SeqCst) > 0 {
            self.fail_metrics.fetch_sub(1, Ordering::SeqCst);
            return Err(NewRelicAgentError::Metric {
                message: "metric boom".to_owned(),
            });
        }
        self.metrics
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((name.to_owned(), value));
        Ok(())
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum AgentKind {
    #[default]
    Installed,
    Missing,
}

pub(super) struct FakeAgent {
    register_error: Option<String>,
    application: Option<Arc<FakeApplication>>,
    registrations: Arc<Mutex<Vec<Duration>>>,
}

impl NewRelicAgent for FakeAgent {
    fn register_application(&self, timeout: Duration) -> Result<(), NewRelicAgentError> {
        self.registrations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(timeout);
        self.register_error.as_ref().map_or(Ok(()), |message| {
            Err(NewRelicAgentError::Registration {
                message: message.clone(),
            })
        })
    }

    fn application(&self) -> Option<Arc<dyn NewRelicApplication>> {
        let application = self.application.as_ref()?;
        let source: Arc<FakeApplication> = Arc::clone(application);
        let callback: Arc<dyn NewRelicApplication> = source;
        Some(callback)
    }
}

#[derive(Default)]
pub(super) struct CountingCounter {
    calls: AtomicUsize,
    names: Mutex<Vec<String>>,
}

impl CountingCounter {
    pub(super) fn calls(&self) -> Vec<String> {
        self.names
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl CallbackFailureCounter for CountingCounter {
    fn increment_callback_logging_failure(&self, callback_name: &str) {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.names
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(callback_name.to_owned());
    }
}

pub(super) struct TestClock {
    now: Mutex<Duration>,
}

impl TestClock {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self {
            now: Mutex::new(Duration::from_secs(CLOCK_START)),
        })
    }

    pub(super) fn now(&self) -> Duration {
        *self.now.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn advance(&self, seconds: u64) {
        let mut now = self.now.lock().unwrap_or_else(PoisonError::into_inner);
        *now = now.saturating_add(Duration::from_secs(seconds));
    }
}

pub(super) struct SequenceIds {
    next: AtomicU64,
}

impl SequenceIds {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self {
            next: AtomicU64::new(1),
        })
    }
}

impl UuidSource for SequenceIds {
    fn next_id(&self) -> Uuid {
        Uuid::from_u128(u128::from(self.next.fetch_add(1, Ordering::SeqCst)))
    }
}

pub(super) struct Fixture {
    pub application: Arc<FakeApplication>,
    pub agent_kind: AgentKind,
    pub register_error: Option<String>,
    pub environment: BTreeMap<String, String>,
    pub init: NewRelicInit,
    pub clock: Arc<TestClock>,
    pub ids: Arc<SequenceIds>,
    pub backoff: Arc<MetricBackoff>,
    pub version: Option<String>,
    pub counter: Arc<CountingCounter>,
    pub registrations: Arc<Mutex<Vec<Duration>>>,
}

impl Default for Fixture {
    fn default() -> Self {
        Self {
            application: FakeApplication::new(true),
            agent_kind: AgentKind::Installed,
            register_error: None,
            environment: environment(&[
                ("NEW_RELIC_LICENSE_KEY", "license-key"),
                ("NEW_RELIC_APP_NAME", "my-app"),
            ]),
            init: NewRelicInit::default(),
            clock: TestClock::new(),
            ids: SequenceIds::new(),
            backoff: Arc::new(MetricBackoff::default()),
            version: Some("1.2.3".to_owned()),
            counter: Arc::new(CountingCounter::default()),
            registrations: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

pub(super) fn default_fixture() -> Fixture {
    Fixture::default()
}

pub(super) fn call_kwargs() -> CallKwargs<'static> {
    CallKwargs::default()
}

pub(super) fn fallback_kwargs() -> CallKwargs<'static> {
    CallKwargs {
        messages: None,
        model: Some("gpt-4o"),
        litellm_call_id: None,
        custom_llm_provider: None,
        headers: None,
        optional_params: None,
        llm_api_duration_ms: None,
        start_time: Some(Moment::Integer(1_704_067_200)),
        end_time: Some(Moment::Integer(1_704_067_202)),
    }
}

pub(super) fn fallback_response() -> IndexMap<String, Json> {
    object(
        r#"{
            "id": "chatcmpl-9",
            "model": "gpt-4o-mini",
            "choices": [{"finish_reason": "length", "message": {"role": "assistant", "content": "hi"}}],
            "usage": {"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3}
        }"#,
    )
}

impl Fixture {
    pub(super) fn build(&self) -> Arc<NewRelicLogger> {
        let agent: Option<Arc<dyn NewRelicAgent>> = match self.agent_kind {
            AgentKind::Installed => {
                let fake = FakeAgent {
                    register_error: self.register_error.clone(),
                    application: Some(Arc::clone(&self.application)),
                    registrations: Arc::clone(&self.registrations),
                };
                let installed: Arc<dyn NewRelicAgent> = Arc::new(fake);
                Some(installed)
            }
            AgentKind::Missing => None,
        };
        let clock = Arc::clone(&self.clock);
        let shared_clock: Clock = Arc::new(move || clock.now());
        let source_ids: Arc<SequenceIds> = Arc::clone(&self.ids);
        let ids: Arc<dyn UuidSource> = source_ids;
        let source_counter: Arc<CountingCounter> = Arc::clone(&self.counter);
        let counter: Arc<dyn CallbackFailureCounter + Send + Sync> = source_counter;
        let deps = NewRelicDeps {
            agent,
            environment: Arc::new(self.environment.clone()),
            clock: shared_clock,
            ids,
            backoff: Arc::clone(&self.backoff),
            version: self.version.clone(),
            failure_counters: vec![counter],
        };
        NewRelicLogger::new(&self.init, deps)
    }

    pub(super) fn turn_off(&mut self, turn_off: bool) {
        self.init.params = NewRelicInitParams {
            turn_off_message_logging: turn_off,
        };
    }

    pub(super) fn registrations(&self) -> Vec<Duration> {
        self.registrations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Clone, Default)]
pub(super) struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    pub(super) fn text(&self) -> String {
        let bytes = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        String::from_utf8(bytes).unwrap()
    }
}

impl Write for LogBuffer {
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

impl<'writer> MakeWriter<'writer> for LogBuffer {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

pub(super) fn capture_subscriber() -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(Level::TRACE)
        .with_writer(buffer.clone())
        .finish();
    let guard: DefaultGuard = set_default(subscriber);
    (buffer, guard)
}

pub(super) fn capture_logs<T, F>(action: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    let (buffer, guard) = capture_subscriber();
    let result = action();
    drop(guard);
    (result, buffer.text())
}
