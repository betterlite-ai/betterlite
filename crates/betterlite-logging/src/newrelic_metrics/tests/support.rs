use std::io::{Result as IoResult, Write};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use rust_decimal::Decimal;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use crate::fields::payload::StandardLoggingPayload;
use crate::newrelic_metrics::aggregation::build_metric_payload;
use crate::newrelic_metrics::records::NewRelicMetricRecord;
use crate::newrelic_metrics::wire::{
    CountValue, MetricAttributes, NewRelicMetric, NewRelicMetricEnvelope, NewRelicSummaryValue,
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

pub(super) fn capture_warnings() -> (WarningBuffer, DefaultGuard) {
    let buffer = WarningBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(Level::WARN)
        .with_writer(buffer.clone())
        .finish();
    let guard = set_default(subscriber);
    (buffer, guard)
}
