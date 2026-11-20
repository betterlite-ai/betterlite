#![cfg(test)]

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use betterlite_prometheus::collector_factory::collector::{Collector, MetricUpdate, MetricValue};
use betterlite_prometheus::label_value_record::UserApiKeyLabelValues;
use rust_decimal::Decimal;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use crate::end_user::EndUserCostTrackingSettings;
use crate::prometheus_helpers::bounded_series_tracker::{
    BoundedPrometheusSeriesTracker, ChildRemoval, RemovableMetric, SeriesLimits,
};
use crate::prometheus_label_projection::label_factory_context::PrometheusLabelFactoryContext;
use crate::prometheus_label_schema::PrometheusLabelSettings;
use crate::prometheus_latency::guardrail_series::GuardrailSeries;
use crate::prometheus_latency::input_sequence_length::{
    CombinedUsage, InputLengthInputs, UsageMapping,
};
use crate::prometheus_latency::labels::LabelInputs;
use crate::prometheus_latency::latency_histograms::{
    LatencyHistogram, LatencyHistograms, LatencyObservation, LatencyTimestamps, MetricHandle,
    set_latency_metrics,
};

pub(crate) type EventLog = Arc<Mutex<Vec<Recorded>>>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Recorded {
    Update {
        metric: &'static str,
        labels: Vec<(String, String)>,
        update: MetricUpdate,
    },
    Removal {
        metric: &'static str,
        label_values: Vec<Option<String>>,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct RecordingMetric {
    pub(crate) name: &'static str,
    pub(crate) labels: Vec<(String, String)>,
    pub(crate) events: EventLog,
}

impl RecordingMetric {
    pub(crate) fn with_log(name: &'static str, events: &EventLog) -> Self {
        Self {
            name,
            labels: Vec::new(),
            events: Arc::clone(events),
        }
    }
}

impl Collector for RecordingMetric {
    fn labels(&self, labels: &[(&str, &str)]) -> Self {
        Self {
            name: self.name,
            labels: owned_pairs(labels),
            events: Arc::clone(&self.events),
        }
    }

    fn update(&self, update: MetricUpdate) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded::Update {
                metric: self.name,
                labels: self.labels.clone(),
                update,
            });
    }
}

impl RemovableMetric for RecordingMetric {
    type Error = Infallible;

    fn remove_child(&self, label_values: &[Option<String>]) -> Result<ChildRemoval, Infallible> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded::Removal {
                metric: self.name,
                label_values: label_values.to_vec(),
            });
        Ok(ChildRemoval::Removed)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the guardrail series rejected the labels")]
pub(crate) struct RejectedLabels;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FailurePoint {
    NoFailure,
    Observe,
    Increment,
}

#[derive(Clone, Debug)]
pub(crate) struct FailingSeries {
    pub(crate) name: &'static str,
    pub(crate) events: EventLog,
    pub(crate) failure: FailurePoint,
}

impl FailingSeries {
    pub(crate) fn with_log(name: &'static str, events: &EventLog, failure: FailurePoint) -> Self {
        Self {
            name,
            events: Arc::clone(events),
            failure,
        }
    }

    fn record(&self, labels: &[(&str, &str)], update: MetricUpdate) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded::Update {
                metric: self.name,
                labels: owned_pairs(labels),
                update,
            });
    }
}

impl GuardrailSeries for FailingSeries {
    type Error = RejectedLabels;

    fn observe(&self, labels: &[(&str, &str)], seconds: Decimal) -> Result<(), Self::Error> {
        if self.failure == FailurePoint::Observe {
            return Err(RejectedLabels);
        }
        self.record(labels, MetricUpdate::Observe(MetricValue::Finite(seconds)));
        Ok(())
    }

    fn increment(&self, labels: &[(&str, &str)]) -> Result<(), Self::Error> {
        if self.failure == FailurePoint::Increment {
            return Err(RejectedLabels);
        }
        self.record(labels, MetricUpdate::Inc(MetricValue::Finite(Decimal::ONE)));
        Ok(())
    }
}

pub(crate) fn owned_pairs(labels: &[(&str, &str)]) -> Vec<(String, String)> {
    labels
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(crate) fn dec(value: i64, scale: u32) -> Decimal {
    Decimal::new(value, scale)
}

pub(crate) fn supported(labels: &[&str]) -> Vec<String> {
    labels.iter().map(|label| (*label).to_owned()).collect()
}

pub(crate) fn end_user_settings() -> EndUserCostTrackingSettings {
    EndUserCostTrackingSettings {
        disable_end_user_cost_tracking: None,
        enable_prometheus_only: Some(true),
    }
}

pub(crate) fn label_values(end_user: Option<&str>, length: Option<&str>) -> UserApiKeyLabelValues {
    UserApiKeyLabelValues {
        end_user: end_user.map(ToOwned::to_owned),
        user: None,
        user_email: None,
        user_alias: None,
        hashed_api_key: None,
        api_key_alias: None,
        team: None,
        team_alias: None,
        model_group: None,
        requested_model: None,
        model: Some("gpt-4o".to_owned()),
        litellm_model_name: None,
        tags: Vec::new(),
        custom_metadata_labels: BTreeMap::new(),
        model_id: None,
        api_base: None,
        api_provider: None,
        exception_status: None,
        exception_class: None,
        rate_limit_category: None,
        rate_limit_type: None,
        status_code: None,
        fallback_model: None,
        route: None,
        client_ip: None,
        user_agent: None,
        stream: None,
        org_id: None,
        org_alias: None,
        mcp_tool_name: None,
        mcp_server_name: None,
        service_tier: None,
        input_sequence_length: length.map(ToOwned::to_owned),
    }
}

pub(crate) fn four_metrics(events: &EventLog) -> [RecordingMetric; 4] {
    [
        RecordingMetric::with_log("ttft", events),
        RecordingMetric::with_log("api", events),
        RecordingMetric::with_log("total", events),
        RecordingMetric::with_log("queue", events),
    ]
}

pub(crate) fn observation(
    timestamps: LatencyTimestamps,
    stream: Option<bool>,
    queue_time_seconds: Option<Decimal>,
    input_sequence_length: Option<&str>,
) -> LatencyObservation<'_> {
    LatencyObservation {
        timestamps,
        now: dec(20000, 1),
        now_monotonic: Instant::now(),
        stream,
        queue_time_seconds,
        input_sequence_length,
    }
}

pub(crate) fn no_limits() -> SeriesLimits {
    SeriesLimits {
        max_series: None,
        ttl: None,
        cleanup_interval: None,
    }
}

pub(crate) fn latency_histograms<'labels>(
    metrics: [&'labels RecordingMetric; 4],
    supported_labels: &'labels [String],
    tracker: &'labels BoundedPrometheusSeriesTracker,
    limits: SeriesLimits,
) -> LatencyHistograms<'labels, RecordingMetric> {
    let [ttft, api_latency, total_latency, queue_time] = metrics;
    LatencyHistograms {
        time_to_first_token: latency_histogram(
            ttft,
            supported_labels,
            "litellm_llm_api_time_to_first_token_metric",
            tracker,
            limits,
        ),
        api_latency: latency_histogram(
            api_latency,
            supported_labels,
            "litellm_llm_api_latency_metric",
            tracker,
            limits,
        ),
        total_latency: latency_histogram(
            total_latency,
            supported_labels,
            "litellm_request_total_latency_metric",
            tracker,
            limits,
        ),
        queue_time: latency_histogram(
            queue_time,
            supported_labels,
            "litellm_request_queue_time_seconds",
            tracker,
            limits,
        ),
    }
}

fn latency_histogram<'labels>(
    metric: &'labels RecordingMetric,
    supported_labels: &'labels [String],
    metric_name: &'static str,
    tracker: &'labels BoundedPrometheusSeriesTracker,
    limits: SeriesLimits,
) -> LatencyHistogram<'labels, RecordingMetric> {
    LatencyHistogram {
        handle: MetricHandle {
            metric,
            supported_labels,
        },
        metric_name,
        tracker,
        limits,
    }
}

pub(crate) fn recorded_events(
    supported_labels: &[&str],
    values: &UserApiKeyLabelValues,
    context: Option<&PrometheusLabelFactoryContext>,
    limits: SeriesLimits,
    observed_request: &LatencyObservation<'_>,
) -> Vec<Recorded> {
    let events = EventLog::default();
    let metrics = four_metrics(&events);
    let labels = supported(supported_labels);
    let tracker = BoundedPrometheusSeriesTracker::default();
    let histograms = latency_histograms(
        [&metrics[0], &metrics[1], &metrics[2], &metrics[3]],
        &labels,
        &tracker,
        limits,
    );
    let settings = label_settings();
    let inputs = LabelInputs {
        values,
        settings: &settings,
        end_user_settings: end_user_settings(),
        context,
    };
    set_latency_metrics(&histograms, observed_request, &inputs).unwrap();
    recorded(&events)
}

pub(crate) fn observe_with(
    histograms: &LatencyHistograms<'_, RecordingMetric>,
    values: &UserApiKeyLabelValues,
    observed_request: &LatencyObservation<'_>,
) {
    let settings = label_settings();
    let inputs = LabelInputs {
        values,
        settings: &settings,
        end_user_settings: end_user_settings(),
        context: None,
    };
    set_latency_metrics(histograms, observed_request, &inputs).unwrap();
}

pub(crate) fn end_user_cap_events(requests: [&UserApiKeyLabelValues; 2]) -> Vec<Recorded> {
    let events = EventLog::default();
    let metrics = four_metrics(&events);
    let labels = supported(&["model", "end_user"]);
    let tracker = BoundedPrometheusSeriesTracker::default();
    let limits = SeriesLimits {
        max_series: Some(1),
        ttl: None,
        cleanup_interval: None,
    };
    let histograms = latency_histograms(
        [&metrics[0], &metrics[1], &metrics[2], &metrics[3]],
        &labels,
        &tracker,
        limits,
    );
    let observed_request = observation(
        LatencyTimestamps {
            start_time: None,
            end_time: Some(dec(1010, 1)),
            api_call_start: Some(dec(1000, 1)),
            completion_start: None,
        },
        None,
        None,
        None,
    );
    for values in requests {
        observe_with(&histograms, values, &observed_request);
    }
    recorded(&events)
}

pub(crate) fn recorded(events: &EventLog) -> Vec<Recorded> {
    events
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

pub(crate) fn observed(metric: &'static str, labels: &[(&str, &str)], value: Decimal) -> Recorded {
    Recorded::Update {
        metric,
        labels: owned_pairs(labels),
        update: MetricUpdate::Observe(MetricValue::Finite(value)),
    }
}

pub(crate) fn incremented(metric: &'static str, labels: &[(&str, &str)]) -> Recorded {
    Recorded::Update {
        metric,
        labels: owned_pairs(labels),
        update: MetricUpdate::Inc(MetricValue::Finite(Decimal::ONE)),
    }
}

pub(crate) fn removed(metric: &'static str, label_values: &[Option<&str>]) -> Recorded {
    Recorded::Removal {
        metric,
        label_values: label_values
            .iter()
            .map(|value| value.map(ToOwned::to_owned))
            .collect(),
    }
}

pub(crate) fn label_value(event: &Recorded, name: &str) -> Option<String> {
    match event {
        Recorded::Update { labels, .. } => labels
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone()),
        Recorded::Removal { .. } => None,
    }
}

pub(crate) fn bucket_inputs(
    prompt_tokens: Option<i64>,
    combined_usage: Option<CombinedUsage>,
    response_has_usage: bool,
    metadata_usage: UsageMapping,
    response_object: UsageMapping,
) -> InputLengthInputs {
    InputLengthInputs {
        prompt_tokens,
        combined_usage,
        response_has_usage,
        metadata_usage,
        response_object,
    }
}

pub(crate) fn combined(upstream_total_tokens: Option<i64>) -> CombinedUsage {
    CombinedUsage {
        upstream_total_tokens,
    }
}

pub(crate) fn mapping(prompt_tokens: Option<i64>) -> UsageMapping {
    UsageMapping::Mapping { prompt_tokens }
}

#[derive(Clone, Default)]
pub(crate) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub(crate) fn text(&self) -> String {
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

pub(crate) fn capture_logs() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(move || writer.clone())
        .finish();
    (captured, set_default(subscriber))
}

pub(crate) fn label_settings() -> PrometheusLabelSettings {
    PrometheusLabelSettings::with_defaults()
}
