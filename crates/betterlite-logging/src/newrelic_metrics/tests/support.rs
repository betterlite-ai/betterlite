use std::collections::VecDeque;
use std::io::{Read as _, Result as IoResult, Write};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use flate2::read::GzDecoder;
use http::HeaderMap;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use secrecy::SecretString;
use serde::Deserialize;
use tokio::task::yield_now;
use tokio::time::{advance, pause, timeout};
use tokio_util::task::TaskTracker;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use crate::callback_logger::event::LoggerEvent;
use crate::fields::payload::StandardLoggingPayload;
use crate::generic_api_logger::generic_logger::retry::RetrySleeper;
use crate::newrelic_metrics::NewRelicMetricsError;
use crate::newrelic_metrics::aggregation::build_metric_payload;
use crate::newrelic_metrics::clock::Clock;
use crate::newrelic_metrics::logger::{NewRelicDeps, NewRelicMetricsLogger};
use crate::newrelic_metrics::queue::push_record;
use crate::newrelic_metrics::records::NewRelicMetricRecord;
use crate::newrelic_metrics::transport::{NewRelicPost, NewRelicSendError, NewRelicTransport};
use crate::newrelic_metrics::wire::{
    CountValue, METRIC_REQUESTS, MetricAttributes, NewRelicMetric, NewRelicMetricEnvelope,
    NewRelicSummaryValue,
};

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
    "endTime": 1704067201.0,
    "completionStartTime": 1704067200.5,
    "response_time": 0.5,
    "model_map_information": {},
    "model": "gpt-4o",
    "model_group": "production",
    "custom_llm_provider": "openai",
    "api_base": "https://api.openai.com",
    "metadata": {
        "user_api_key_team_id": "team-1",
        "user_api_key_team_alias": "Team One",
        "user_api_key_team_max_budget": 10.0,
        "user_api_key_team_spend": 2.5
    },
    "saved_cache_cost": 0.0,
    "request_tags": [],
    "model_parameters": {},
    "hidden_params": {}
}"#;

pub(super) fn payload() -> StandardLoggingPayload {
    serde_json::from_str(PAYLOAD).expect("a valid standard logging payload")
}

pub(super) fn record() -> NewRelicMetricRecord {
    NewRelicMetricRecord {
        team_id: "team-1".to_owned(),
        team_alias: "Team One".to_owned(),
        model_group: "production".to_owned(),
        model: "gpt-4o".to_owned(),
        custom_llm_provider: "openai".to_owned(),
        status: "success".to_owned(),
        response_cost: Decimal::new(5, 1),
        prompt_tokens: 2,
        completion_tokens: 3,
        total_tokens: 5,
        duration_ms: 500.0,
        team_max_budget: Some(Decimal::new(100, 1)),
        team_spend: Some(Decimal::new(25, 1)),
    }
}

// A distinct model keeps each record in its own bucket, so the queue order stays readable
pub(super) fn record_at(index: usize) -> NewRelicMetricRecord {
    let mut record = record();
    record.model = format!("gpt-{index}");
    record.response_cost = Decimal::ZERO;
    record.duration_ms = 100.0_f64;
    record.team_max_budget = None;
    record.team_spend = None;
    record
}

pub(super) fn event(payload: StandardLoggingPayload) -> Arc<LoggerEvent> {
    Arc::new(LoggerEvent {
        payload: Arc::new(payload),
        dynamic_params: Arc::new(Json::Object(IndexMap::new())),
        litellm_metadata: Arc::new(IndexMap::new()),
        stream: false,
        guardrail_snapshot: None,
    })
}

pub(super) fn envelope(
    records: &[NewRelicMetricRecord],
    window_start_ms: u64,
    now_ms: u64,
) -> NewRelicMetricEnvelope {
    build_metric_payload(
        records,
        Duration::from_millis(window_start_ms),
        Duration::from_millis(now_ms),
    )
    .expect("an envelope")
}

pub(super) fn metric<'envelope>(
    envelope: &'envelope NewRelicMetricEnvelope,
    name: &str,
) -> &'envelope NewRelicMetric {
    envelope
        .metrics
        .iter()
        .find(|metric| metric_name(metric) == name)
        .expect("a metric with that name")
}

pub(super) fn metric_name(metric: &NewRelicMetric) -> &'static str {
    match metric {
        NewRelicMetric::Count { name, .. }
        | NewRelicMetric::Gauge { name, .. }
        | NewRelicMetric::Summary { name, .. } => name,
    }
}

pub(super) fn attributes(metric: &NewRelicMetric) -> &MetricAttributes {
    match metric {
        NewRelicMetric::Count { attributes, .. }
        | NewRelicMetric::Gauge { attributes, .. }
        | NewRelicMetric::Summary { attributes, .. } => attributes,
    }
}

pub(super) fn count_value(metric: &NewRelicMetric) -> &CountValue {
    match metric {
        NewRelicMetric::Count { value, .. } => value,
        NewRelicMetric::Gauge { .. } | NewRelicMetric::Summary { .. } => panic!("a count"),
    }
}

pub(super) fn float_count(envelope: &NewRelicMetricEnvelope, name: &str) -> f64 {
    let CountValue::Float(value) = count_value(metric(envelope, name)) else {
        panic!("a float count metric");
    };
    *value
}

pub(super) fn decimal_count(envelope: &NewRelicMetricEnvelope, name: &str) -> Decimal {
    let CountValue::Decimal(value) = count_value(metric(envelope, name)) else {
        panic!("a decimal count metric");
    };
    *value
}

pub(super) fn summary(metric: &NewRelicMetric) -> &NewRelicSummaryValue {
    let NewRelicMetric::Summary { value, .. } = metric else {
        panic!("a summary metric");
    };
    value
}

pub(super) fn metric_names(envelope: &NewRelicMetricEnvelope) -> Vec<&'static str> {
    envelope.metrics.iter().map(metric_name).collect()
}

pub(super) fn attribute_at<'envelope>(
    envelope: &'envelope NewRelicMetricEnvelope,
    index: usize,
    key: &str,
) -> &'envelope str {
    let metric = envelope.metrics.get(index).expect("a metric at that index");
    attributes(metric)
        .get(key)
        .map(String::as_str)
        .expect("an attribute with that key")
}

#[derive(Clone)]
pub(super) struct ScriptedClock {
    values: Arc<Mutex<VecDeque<f64>>>,
    fallback: f64,
}

impl ScriptedClock {
    pub(super) fn new(seconds: &[f64]) -> Self {
        let fallback = seconds.last().copied().unwrap_or_default();
        Self {
            values: Arc::new(Mutex::new(seconds.iter().copied().collect())),
            fallback,
        }
    }

    pub(super) fn handle(&self) -> Clock {
        let values = Arc::clone(&self.values);
        let fallback = self.fallback;
        Arc::new(move || {
            let seconds = values
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop_front()
                .unwrap_or(fallback);
            Duration::try_from_secs_f64(seconds).expect("a scripted clock instant")
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Reply {
    Status(u16),
    Network,
}

#[derive(Clone)]
pub(super) struct Recorded {
    pub(super) url: &'static str,
    pub(super) headers: HeaderMap,
    pub(super) body: Vec<u8>,
}

pub(super) struct FakeTransport {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<Recorded>>,
    attached: Mutex<Option<Weak<NewRelicMetricsLogger>>>,
    during_send: Mutex<Option<NewRelicMetricRecord>>,
}

impl FakeTransport {
    pub(super) fn attach(&self, logger: &Arc<NewRelicMetricsLogger>) {
        *self.attached.lock().unwrap_or_else(PoisonError::into_inner) =
            Some(Arc::downgrade(logger));
    }

    pub(super) fn push_during_send(&self, record: NewRelicMetricRecord) {
        *self
            .during_send
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(record);
    }

    #[must_use]
    pub(super) fn requests(&self) -> Vec<Recorded> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    #[must_use]
    pub(super) fn payloads(&self) -> Vec<Vec<SentEnvelope>> {
        self.requests()
            .iter()
            .map(|request| {
                serde_json::from_slice(&gunzip(&request.body)).expect("metric envelopes")
            })
            .collect()
    }

    fn append_during_send(&self) {
        let queued = self
            .during_send
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        let Some(record) = queued else {
            return;
        };
        let attached = self
            .attached
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .and_then(Weak::upgrade);
        if let Some(logger) = attached {
            push_record(&logger, record);
        }
    }
}

// A network failure is modelled with the typed transport error, since a reqwest error cannot be built here
#[async_trait]
impl NewRelicTransport for FakeTransport {
    async fn post(&self, request: NewRelicPost) -> Result<u16, NewRelicSendError> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded {
                url: request.url,
                headers: request.headers,
                body: request.body,
            });
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .expect("a scripted Metric API reply");
        self.append_during_send();
        match reply {
            Reply::Status(status) => Ok(status),
            Reply::Network => Err(NewRelicSendError::Request(TransportError::Rejected(
                HttpError::Timeout,
            ))),
        }
    }
}

#[derive(Default)]
pub(super) struct RecordingSleeper {
    sleeps: Mutex<Vec<Duration>>,
}

impl RecordingSleeper {
    #[must_use]
    pub(super) fn sleeps(&self) -> Vec<Duration> {
        self.sleeps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl RetrySleeper for RecordingSleeper {
    async fn sleep(&self, duration: Duration) {
        self.sleeps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(duration);
    }
}

pub(super) struct Fixture {
    pub(super) transport: Arc<FakeTransport>,
    pub(super) clock: ScriptedClock,
    pub(super) sleeper: Arc<RecordingSleeper>,
    pub(super) tasks: TaskTracker,
    pub(super) api_key: String,
    pub(super) region: Option<String>,
}

impl Fixture {
    pub(super) fn new() -> Self {
        Self::new_with_replies(&[Reply::Status(202)])
    }

    pub(super) fn new_with_replies(replies: &[Reply]) -> Self {
        Self {
            transport: Arc::new(FakeTransport {
                replies: Mutex::new(replies.iter().copied().collect()),
                requests: Mutex::new(Vec::new()),
                attached: Mutex::new(None),
                during_send: Mutex::new(None),
            }),
            clock: ScriptedClock::new(&[1_000.0_f64]),
            sleeper: Arc::new(RecordingSleeper::default()),
            tasks: TaskTracker::new(),
            api_key: "team-key".to_owned(),
            region: None,
        }
    }

    #[must_use]
    pub(super) fn with_key(mut self, api_key: &str) -> Self {
        self.api_key = api_key.to_owned();
        self
    }

    #[must_use]
    pub(super) fn with_region(mut self, region: &str) -> Self {
        self.region = Some(region.to_owned());
        self
    }

    pub(super) fn build_result(&self) -> Result<Arc<NewRelicMetricsLogger>, NewRelicMetricsError> {
        let deps = NewRelicDeps {
            transport: Arc::<FakeTransport>::clone(&self.transport),
            clock: self.clock.handle(),
            sleeper: Arc::<RecordingSleeper>::clone(&self.sleeper),
            flush_interval: Duration::from_secs(5),
            tasks: self.tasks.clone(),
        };
        let built = NewRelicMetricsLogger::new(
            SecretString::from(self.api_key.clone()),
            self.region.as_deref(),
            deps,
        );
        if let Ok(logger) = &built {
            self.transport.attach(logger);
        }
        built
    }

    // Ends the periodic flush task so a later stop() has only its own drain left to deliver
    pub(super) async fn end_periodic_flush(&self) {
        pause();
        yield_now().await;
        self.tasks.close();
        advance(Duration::from_secs(5)).await;
        timeout(Duration::from_secs(20), self.tasks.wait())
            .await
            .expect("the periodic flush ends once the tracker closes");
    }

    pub(super) async fn wait_for_tasks(&self) {
        self.tasks.close();
        timeout(Duration::from_secs(20), self.tasks.wait())
            .await
            .expect("every tracked task ends once the tracker closes");
    }

    pub(super) fn build(&self) -> Arc<NewRelicMetricsLogger> {
        self.build_result().expect("a New Relic metrics logger")
    }
}

impl Default for Fixture {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Deserialize)]
pub(super) struct SentCommon {
    pub(super) timestamp: u64,
    #[serde(rename = "interval.ms")]
    pub(super) interval_ms: u64,
}

#[derive(Deserialize)]
pub(super) struct SentMetric {
    pub(super) name: String,
    pub(super) value: Json,
}

#[derive(Deserialize)]
pub(super) struct SentEnvelope {
    pub(super) common: SentCommon,
    pub(super) metrics: Vec<SentMetric>,
}

#[must_use]
pub(super) fn gunzip(body: &[u8]) -> Vec<u8> {
    let mut decoded = Vec::new();
    GzDecoder::new(body)
        .read_to_end(&mut decoded)
        .expect("a gzip encoded body");
    decoded
}

#[must_use]
pub(super) fn header(request: &Recorded, name: &str) -> String {
    request
        .headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .expect("a request header")
        .to_owned()
}

#[must_use]
pub(super) fn record_count(envelope: &SentEnvelope) -> f64 {
    let metric = envelope
        .metrics
        .iter()
        .find(|metric| metric.name == METRIC_REQUESTS)
        .expect("a requests metric");
    let Json::Number(number) = &metric.value else {
        panic!("a numeric requests count");
    };
    number.as_f64().expect("a numeric requests count")
}

#[derive(Clone, Default)]
pub(super) struct WarningBuffer(Arc<Mutex<Vec<u8>>>);

impl WarningBuffer {
    pub(super) fn text(&self) -> String {
        let bytes = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        String::from_utf8(bytes).expect("utf-8 log output")
    }
}

impl Write for WarningBuffer {
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

impl<'writer> MakeWriter<'writer> for WarningBuffer {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

pub(super) fn capture_at(level: Level) -> (WarningBuffer, DefaultGuard) {
    let buffer = WarningBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(level)
        .with_writer(buffer.clone())
        .finish();
    let guard = set_default(subscriber);
    (buffer, guard)
}

pub(super) fn capture_warnings() -> (WarningBuffer, DefaultGuard) {
    capture_at(Level::WARN)
}

pub(super) fn capture_logs() -> (WarningBuffer, DefaultGuard) {
    capture_at(Level::DEBUG)
}
