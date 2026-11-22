use std::collections::BTreeMap;
use std::convert::Infallible;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_logging::end_user::EndUserCostTrackingSettings;
use betterlite_logging::prometheus_deployment::metric::DeploymentMetric;
use betterlite_logging::prometheus_deployment::metrics::DeploymentMetrics;
use betterlite_logging::prometheus_helpers::bounded_series_tracker::{
    BoundedPrometheusSeriesTracker, ChildRemoval, RemovableMetric, SeriesLimits,
};
use betterlite_logging::prometheus_label_schema::{PrometheusLabelSettings, get_labels};
use betterlite_logging::prometheus_rate_limits::gauge::RateLimitGauges;
use betterlite_logging::prometheus_rate_limits::metric::RateLimitMetric;
use betterlite_prometheus::collector_factory::collector::{Collector, MetricUpdate, MetricValue};
use serde_json::json;

const METRICS: [&str; 13] = [
    "litellm_deployment_state",
    "litellm_deployment_failure_responses",
    "litellm_deployment_total_requests",
    "litellm_deployment_success_responses",
    "litellm_deployment_successful_fallbacks",
    "litellm_deployment_failed_fallbacks",
    "litellm_deployment_tpm_limit",
    "litellm_deployment_rpm_limit",
    "litellm_deployment_latency_per_output_token",
    "litellm_overhead_latency_metric",
    "litellm_overhead_with_guardrails_latency_metric",
    "litellm_remaining_requests_metric",
    "litellm_remaining_tokens_metric",
];

#[derive(Clone, Debug)]
struct Event {
    metric: &'static str,
    labels: Vec<(String, String)>,
    kind: &'static str,
    value: String,
}

#[derive(Clone)]
pub(super) struct ProbeMetric {
    metric: &'static str,
    labels: Vec<(String, String)>,
    events: Arc<Mutex<Vec<Event>>>,
}

impl Collector for ProbeMetric {
    fn labels(&self, labels: &[(&str, &str)]) -> Self {
        Self {
            metric: self.metric,
            labels: labels
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
            events: Arc::clone(&self.events),
        }
    }

    fn update(&self, update: MetricUpdate) {
        let (kind, value) = match update {
            MetricUpdate::Inc(value) => ("inc", amount_text(value)),
            MetricUpdate::Set(value) => ("set", amount_text(value)),
            MetricUpdate::Observe(value) => ("observe", amount_text(value)),
            MetricUpdate::Remove => ("remove", String::new()),
        };
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Event {
                metric: self.metric,
                labels: self.labels.clone(),
                kind,
                value,
            });
    }
}

impl RemovableMetric for ProbeMetric {
    type Error = Infallible;

    fn remove_child(&self, _label_values: &[Option<String>]) -> Result<ChildRemoval, Self::Error> {
        Ok(ChildRemoval::Unsupported)
    }
}

pub(super) struct Probe {
    events: Arc<Mutex<Vec<Event>>>,
    supported: BTreeMap<String, Vec<String>>,
    settings: PrometheusLabelSettings,
    tracker: BoundedPrometheusSeriesTracker,
    limits: SeriesLimits,
}

impl Probe {
    pub(super) fn new() -> Self {
        let settings = PrometheusLabelSettings::with_defaults();
        let supported = METRICS
            .iter()
            .map(|name| {
                (
                    (*name).to_owned(),
                    get_labels(name, &settings).unwrap_or_default(),
                )
            })
            .collect();
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
            supported,
            settings,
            tracker: BoundedPrometheusSeriesTracker::default(),
            limits: SeriesLimits::default(),
        }
    }

    pub(super) fn events_value(&self) -> Vec<serde_json::Value> {
        let events = self.events.lock().unwrap_or_else(PoisonError::into_inner);
        events
            .iter()
            .map(|event| {
                json!({
                    "metric": event.metric,
                    "labels": event.labels.iter().cloned().collect::<BTreeMap<String, String>>(),
                    "kind": event.kind,
                    "value": event.value,
                })
            })
            .collect()
    }

    fn probe_metric(&self, name: &'static str) -> ProbeMetric {
        ProbeMetric {
            metric: name,
            labels: Vec::new(),
            events: Arc::clone(&self.events),
        }
    }
}

impl RateLimitGauges for Probe {
    type Gauge = ProbeMetric;

    fn gauge(&self, metric: RateLimitMetric) -> ProbeMetric {
        self.probe_metric(metric.name())
    }

    fn supported_labels(&self, metric: RateLimitMetric) -> &[String] {
        self.supported.get(metric.name()).map_or(&[], Vec::as_slice)
    }

    fn tracker(&self) -> &BoundedPrometheusSeriesTracker {
        &self.tracker
    }
}

impl DeploymentMetrics for Probe {
    fn metric(&self, metric: DeploymentMetric) -> ProbeMetric {
        self.probe_metric(metric.name())
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
}

fn amount_text(value: MetricValue) -> String {
    match value {
        MetricValue::Finite(decimal) => decimal.to_string(),
        MetricValue::PositiveInfinity => "Infinity".to_owned(),
        MetricValue::NegativeInfinity => "-Infinity".to_owned(),
        MetricValue::NotANumber => "NaN".to_owned(),
    }
}
