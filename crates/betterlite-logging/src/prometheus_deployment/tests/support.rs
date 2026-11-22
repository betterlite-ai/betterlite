#![cfg(test)]

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use betterlite_error::normalize::class::Class;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_prometheus::collector_factory::collector::{Collector, MetricUpdate, MetricValue};
use betterlite_prometheus::label_value_record::{LabelDumpValue, UserApiKeyLabelValues};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde_json::Number;
use tracing::subscriber::DefaultGuard;
use tracing_subscriber::fmt::MakeWriter;

use crate::end_user::EndUserCostTrackingSettings;
use crate::fields::payload::StandardLoggingPayload;
use crate::prometheus::failure::RequestedModelCatalog;
use crate::prometheus::failure::deployment_labels::{
    DeploymentFailureLabels, DeploymentFailureSources, deployment_failure_labels,
};
use crate::prometheus::failure::exception::LabelException;
use crate::prometheus::failure::exception_name::exception_class_name;
use crate::prometheus::failure::model_cardinality::bounded_requested_model_label;
use crate::prometheus_helpers::bounded_series_tracker::{
    BoundedPrometheusSeriesTracker, ChildRemoval, RemovableMetric, SeriesLimits,
};
use crate::prometheus_label_projection::prometheus_label_factory::LabelSource;
use crate::prometheus_label_schema::{PrometheusLabelSettings, get_labels};
use crate::prometheus_rate_limits::gauge::RateLimitGauges;
use crate::prometheus_rate_limits::metric::RateLimitMetric;

use super::error::PrometheusDeploymentError;
use super::metric::DeploymentMetric;
use super::metrics::{DeploymentMetrics, DeploymentResult, emit_labeled_counter};

pub(crate) type EventLog = Arc<Mutex<Vec<Recorded>>>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Recorded {
    Update {
        metric: &'static str,
        labels: BTreeMap<String, String>,
        update: MetricUpdate,
    },
    Call {
        name: &'static str,
        detail: String,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct ProbeMetric {
    name: &'static str,
    labels: BTreeMap<String, String>,
    events: EventLog,
}

impl Collector for ProbeMetric {
    fn labels(&self, labels: &[(&str, &str)]) -> Self {
        Self {
            name: self.name,
            labels: labels
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
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

impl RemovableMetric for ProbeMetric {
    type Error = Infallible;

    fn remove_child(&self, _label_values: &[Option<String>]) -> Result<ChildRemoval, Self::Error> {
        Ok(ChildRemoval::Unsupported)
    }
}

// one answer per catalog question: recognized, public and pattern route
pub(crate) struct FakeCatalog {
    answers: [bool; 3],
    calls: Mutex<Vec<String>>,
}

impl FakeCatalog {
    pub(crate) fn rejecting() -> Self {
        Self {
            answers: [false; 3],
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl RequestedModelCatalog for FakeCatalog {
    fn is_recognized_model(&self, model: &str) -> bool {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(model.to_owned());
        self.answers[0]
    }

    fn is_team_public_model(&self, _model: &str) -> bool {
        self.answers[1]
    }

    fn matches_pattern_route(&self, _model: &str) -> bool {
        self.answers[2]
    }
}

pub(crate) struct ProbeGauges {
    events: EventLog,
    supported: BTreeMap<String, Vec<String>>,
    settings: PrometheusLabelSettings,
    tracker: BoundedPrometheusSeriesTracker,
    limits: SeriesLimits,
    skip_invalid_key: bool,
    failing: Option<DeploymentMetric>,
    failure_labels_override: Option<DeploymentFailureLabels>,
    exception_class_override: Option<String>,
}

const DEPLOYMENT_METRICS: [DeploymentMetric; 14] = [
    DeploymentMetric::CallbackLoggingFailures,
    DeploymentMetric::CooledDown,
    DeploymentMetric::DeploymentState,
    DeploymentMetric::FailedFallbacks,
    DeploymentMetric::FailureResponses,
    DeploymentMetric::LatencyPerOutputToken,
    DeploymentMetric::ManagedBatchCreated,
    DeploymentMetric::OverheadLatency,
    DeploymentMetric::OverheadWithGuardrails,
    DeploymentMetric::RemainingRequests,
    DeploymentMetric::RemainingTokens,
    DeploymentMetric::SuccessResponses,
    DeploymentMetric::SuccessfulFallbacks,
    DeploymentMetric::TotalRequests,
];

const RATE_LIMIT_METRICS: [RateLimitMetric; 2] = [
    RateLimitMetric::DeploymentTpmLimit,
    RateLimitMetric::DeploymentRpmLimit,
];

fn supported_metrics(settings: &PrometheusLabelSettings) -> BTreeMap<String, Vec<String>> {
    let mut supported = BTreeMap::new();
    for metric in DEPLOYMENT_METRICS {
        supported.insert(
            metric.name().to_owned(),
            get_labels(metric.name(), settings).unwrap(),
        );
    }
    for metric in RATE_LIMIT_METRICS {
        supported.insert(
            metric.name().to_owned(),
            get_labels(metric.name(), settings).unwrap(),
        );
    }
    supported
}

impl ProbeGauges {
    pub(crate) fn new() -> Self {
        let settings = PrometheusLabelSettings::with_defaults();
        let supported = supported_metrics(&settings);
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
            supported,
            settings,
            tracker: BoundedPrometheusSeriesTracker::default(),
            limits: SeriesLimits::default(),
            skip_invalid_key: false,
            failing: None,
            failure_labels_override: None,
            exception_class_override: None,
        }
    }

    pub(crate) fn skipping_invalid_keys(mut self) -> Self {
        self.skip_invalid_key = true;
        self
    }

    pub(crate) fn failing_on(mut self, metric: DeploymentMetric) -> Self {
        self.failing = Some(metric);
        self
    }

    pub(crate) fn with_failure_labels(mut self, labels: DeploymentFailureLabels) -> Self {
        self.failure_labels_override = Some(labels);
        self
    }

    pub(crate) fn with_exception_class(mut self, class: &str) -> Self {
        self.exception_class_override = Some(class.to_owned());
        self
    }

    pub(crate) fn replacing_labels(mut self, metric: &str, labels: &[&str]) -> Self {
        self.supported.insert(
            metric.to_owned(),
            labels.iter().map(|label| (*label).to_owned()).collect(),
        );
        self
    }

    pub(crate) fn with_custom_tags(mut self, tags: &[&str]) -> Self {
        self.settings.custom_tags = tags.iter().map(|tag| (*tag).to_owned()).collect();
        self.supported = supported_metrics(&self.settings);
        self
    }

    pub(crate) fn defaults(&self) -> DefaultProbeGauges<'_> {
        DefaultProbeGauges(self)
    }

    pub(crate) fn events(&self) -> Vec<Recorded> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn updates(&self) -> Vec<Recorded> {
        self.events()
            .into_iter()
            .filter(|event| matches!(event, Recorded::Update { .. }))
            .collect()
    }

    pub(crate) fn calls(&self) -> Vec<Recorded> {
        self.events()
            .into_iter()
            .filter(|event| matches!(event, Recorded::Call { .. }))
            .collect()
    }

    fn record(&self, name: &'static str, detail: &str) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded::Call {
                name,
                detail: detail.to_owned(),
            });
    }
}

impl RateLimitGauges for ProbeGauges {
    type Gauge = ProbeMetric;

    fn gauge(&self, metric: RateLimitMetric) -> ProbeMetric {
        ProbeMetric {
            name: metric.name(),
            labels: BTreeMap::new(),
            events: Arc::clone(&self.events),
        }
    }

    fn supported_labels(&self, metric: RateLimitMetric) -> &[String] {
        self.supported.get(metric.name()).map_or(&[], Vec::as_slice)
    }

    fn tracker(&self) -> &BoundedPrometheusSeriesTracker {
        &self.tracker
    }
}

impl DeploymentMetrics for ProbeGauges {
    fn metric(&self, metric: DeploymentMetric) -> ProbeMetric {
        ProbeMetric {
            name: metric.name(),
            labels: BTreeMap::new(),
            events: Arc::clone(&self.events),
        }
    }

    fn labels_for(&self, metric: DeploymentMetric) -> &[String] {
        self.supported.get(metric.name()).map_or(&[], Vec::as_slice)
    }

    fn settings(&self) -> &PrometheusLabelSettings {
        &self.settings
    }

    fn end_user_settings(&self) -> EndUserCostTrackingSettings {
        EndUserCostTrackingSettings::default()
    }

    fn limits(&self) -> &SeriesLimits {
        &self.limits
    }

    fn failure_labels(&self, sources: &DeploymentFailureSources<'_>) -> DeploymentFailureLabels {
        self.record("failure_labels", "");
        self.failure_labels_override
            .clone()
            .unwrap_or_else(|| deployment_failure_labels(sources))
    }

    fn bounded_model_label(
        &self,
        requested_model: Option<&Json>,
        router_originated: bool,
        catalog: Option<&dyn RequestedModelCatalog>,
    ) -> Option<String> {
        let detail = match requested_model {
            Some(Json::Text(text)) => text.clone(),
            Some(Json::Null) => "null".to_owned(),
            Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
                "other".to_owned()
            }
            None => "absent".to_owned(),
        };
        self.record("bounded_model_label", &detail);
        bounded_requested_model_label(requested_model, router_originated, catalog)
    }

    fn exception_class(&self, exception: &LabelException) -> String {
        self.record("exception_class", &exception.class_name);
        self.exception_class_override
            .clone()
            .unwrap_or_else(|| exception_class_name(exception))
    }

    fn skip_invalid_key(
        &self,
        _enum_status_code: Option<&str>,
        _exception: Option<&LabelException>,
        _kwargs_exception: Option<&LabelException>,
    ) -> bool {
        self.record("skip_invalid_key", "");
        self.skip_invalid_key
    }

    fn increment_counter(
        &self,
        metric: DeploymentMetric,
        values: &UserApiKeyLabelValues,
        source: LabelSource<'_>,
        now: Instant,
    ) -> DeploymentResult<Self> {
        if self.failing == Some(metric) {
            return Err(PrometheusDeploymentError::Overflow);
        }
        emit_labeled_counter(self, metric, values, source, now)
    }
}

// the probe without its trait overrides, so a test runs the trait's default methods
pub(crate) struct DefaultProbeGauges<'probe>(&'probe ProbeGauges);

impl RateLimitGauges for DefaultProbeGauges<'_> {
    type Gauge = ProbeMetric;

    fn gauge(&self, metric: RateLimitMetric) -> ProbeMetric {
        self.0.gauge(metric)
    }

    fn supported_labels(&self, metric: RateLimitMetric) -> &[String] {
        self.0.supported_labels(metric)
    }

    fn tracker(&self) -> &BoundedPrometheusSeriesTracker {
        self.0.tracker()
    }
}

impl DeploymentMetrics for DefaultProbeGauges<'_> {
    fn metric(&self, metric: DeploymentMetric) -> ProbeMetric {
        self.0.metric(metric)
    }

    fn labels_for(&self, metric: DeploymentMetric) -> &[String] {
        self.0.labels_for(metric)
    }

    fn settings(&self) -> &PrometheusLabelSettings {
        self.0.settings()
    }

    fn end_user_settings(&self) -> EndUserCostTrackingSettings {
        self.0.end_user_settings()
    }

    fn limits(&self) -> &SeriesLimits {
        self.0.limits()
    }
}

pub(crate) struct Fixture {
    pub(crate) values: UserApiKeyLabelValues,
}

impl Fixture {
    pub(crate) fn new(pairs: &[(&str, &str)]) -> Self {
        Self {
            values: label_values(pairs),
        }
    }
}

pub(crate) fn label_values(pairs: &[(&str, &str)]) -> UserApiKeyLabelValues {
    let entries: Vec<(String, LabelDumpValue)> = pairs
        .iter()
        .map(|(name, value)| {
            (
                (*name).to_owned(),
                LabelDumpValue::String((*value).to_owned()),
            )
        })
        .collect();
    UserApiKeyLabelValues::from_entries(&entries)
}

pub(crate) fn number(literal: &str) -> Json {
    Json::Number(literal.parse::<Number>().unwrap())
}

pub(crate) fn json_object(entries: &[(&str, Json)]) -> JsonObject {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn status_fields() -> Json {
    Json::Object(IndexMap::from([
        (
            "llm_api_status".to_owned(),
            Json::Text("success".to_owned()),
        ),
        (
            "guardrail_status".to_owned(),
            Json::Text("not_run".to_owned()),
        ),
    ]))
}

fn base_document() -> IndexMap<String, Json> {
    let entries = [
        ("id", Json::Text("payload".to_owned())),
        ("trace_id", Json::Text("trace".to_owned())),
        ("session_id", Json::Text("session".to_owned())),
        ("call_type", Json::Text("completion".to_owned())),
        ("status", Json::Text("success".to_owned())),
        ("status_fields", status_fields()),
        ("startTime", number("0")),
        ("endTime", number("0")),
        ("completionStartTime", number("0")),
        ("response_time", number("0")),
        ("response_cost", number("0")),
        ("model_map_information", Json::Object(IndexMap::new())),
        ("model", Json::Text("gpt-4o".to_owned())),
        ("api_base", Json::Text(String::new())),
        ("metadata", Json::Object(IndexMap::new())),
        ("saved_cache_cost", number("0")),
        ("request_tags", Json::List(Vec::new())),
        ("model_parameters", Json::Object(IndexMap::new())),
        ("hidden_params", Json::Object(IndexMap::new())),
        ("total_tokens", number("0")),
        ("prompt_tokens", number("0")),
        ("completion_tokens", number("0")),
    ];
    entries
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect()
}

pub(crate) fn payload_with(extra: &[(&str, Json)]) -> StandardLoggingPayload {
    let mut document = base_document();
    for (name, value) in extra {
        document.insert((*name).to_owned(), value.clone());
    }
    let text = serde_json::to_string(&Json::Object(document)).unwrap();
    serde_json::from_str(&text).unwrap()
}

pub(crate) fn hidden_params(entries: &[(&str, Json)]) -> Json {
    Json::Object(json_object(entries))
}

pub(crate) fn exception(status: Option<&str>, code: Option<&str>) -> LabelException {
    LabelException {
        class_name: "BadRequestError".to_owned(),
        class: Class::Other,
        message: None,
        status_code: status.map(str::to_owned),
        code: code.map(str::to_owned),
        llm_provider: None,
        category: None,
        rate_limit_type: None,
        legacy_class_name: None,
    }
}

pub(crate) fn call(name: &'static str, detail: &str) -> Recorded {
    Recorded::Call {
        name,
        detail: detail.to_owned(),
    }
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

pub(crate) fn capture_logs(level: tracing::Level) -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(level)
        .with_writer(move || writer.clone())
        .finish();
    (captured, tracing::subscriber::set_default(subscriber))
}

pub(crate) fn now() -> Instant {
    Instant::now()
}

pub(crate) fn expected_labels(
    metric: &str,
    overrides: &[(&str, &str)],
) -> BTreeMap<String, String> {
    let mut labels: BTreeMap<String, String> =
        get_labels(metric, &PrometheusLabelSettings::with_defaults())
            .unwrap()
            .into_iter()
            .map(|label| (label, "None".to_owned()))
            .collect();
    for (name, value) in overrides {
        labels.insert((*name).to_owned(), (*value).to_owned());
    }
    labels
}

pub(crate) fn assert_labels(
    events: &[Recorded],
    index: usize,
    metric: &str,
    overrides: &[(&str, &str)],
) {
    assert_eq!(metric_of(&events[index]), metric);
    assert_eq!(
        labels_of(&events[index]),
        &expected_labels(metric, overrides)
    );
}

pub(crate) fn assert_label_pairs(labels: &BTreeMap<String, String>, expected: &[(&str, &str)]) {
    for (name, value) in expected {
        assert_eq!(
            labels.get(*name).map(String::as_str),
            Some(*value),
            "label {name}"
        );
    }
}

pub(crate) fn metric_of(record: &Recorded) -> &'static str {
    match record {
        Recorded::Update { metric, .. } => metric,
        Recorded::Call { .. } => panic!("expected an update, found a call"),
    }
}

pub(crate) fn labels_of(record: &Recorded) -> &BTreeMap<String, String> {
    match record {
        Recorded::Update { labels, .. } => labels,
        Recorded::Call { .. } => panic!("expected an update, found a call"),
    }
}

pub(crate) fn update_of(record: &Recorded) -> MetricUpdate {
    match record {
        Recorded::Update { update, .. } => *update,
        Recorded::Call { .. } => panic!("expected an update, found a call"),
    }
}

pub(crate) fn inc_amount() -> MetricUpdate {
    MetricUpdate::Inc(MetricValue::Finite(Decimal::ONE))
}
