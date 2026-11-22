use betterlite_prometheus::collector_factory::collector::{Collector as _, MetricValue};
use betterlite_prometheus::label_sanitization::sanitize_label_value;
use rust_decimal::Decimal;

use crate::exporter_options::failure_counter::CallbackFailureCounter;
use crate::prometheus_exporter::{Handle, PrometheusCollectors};
use crate::prometheus_helpers::bounded_series_tracker::RemovableMetric;

use super::metric::DeploymentMetric;
use super::metrics::DeploymentMetrics;
use super::requests::{BatchCreatedLabels, CooledDownLabels};
use super::values::MISSING_LABEL_VALUE;

const COOLED_DOWN_LABELS: [&str; 5] = [
    "litellm_model_name",
    "model_id",
    "api_base",
    "api_provider",
    "exception_status",
];

// ast-grep-ignore: no-long-names
pub fn increment_deployment_cooled_down<Gauges>(metrics: &Gauges, labels: CooledDownLabels<'_>)
where
    Gauges: DeploymentMetrics,
{
    let values = [
        labels.litellm_model_name,
        labels.model_id,
        labels.api_base,
        labels.api_provider,
        labels.exception_status,
    ];
    let sanitized: Vec<String> = values
        .into_iter()
        .map(|value| sanitize_label_value(Some(value)).unwrap_or_default())
        .collect();
    let supported = metrics.labels_for(DeploymentMetric::CooledDown);
    let mut pairs: Vec<(&str, &str)> = Vec::with_capacity(COOLED_DOWN_LABELS.len());
    for (name, value) in COOLED_DOWN_LABELS.into_iter().zip(&sanitized) {
        if supported.iter().any(|label| label.as_str() == name) {
            pairs.push((name, value.as_str()));
        }
    }
    metrics.increment_plain(DeploymentMetric::CooledDown, &pairs);
}

// ast-grep-ignore: no-long-names
pub fn increment_callback_logging_failure(counter: &Handle, callback_name: &str) {
    let labels = [("callback_name", callback_name)];
    counter
        .labels(&labels)
        .inc(MetricValue::Finite(Decimal::ONE));
}

pub fn record_managed_batch_created<Gauges>(metrics: &Gauges, labels: BatchCreatedLabels<'_>)
where
    Gauges: DeploymentMetrics,
{
    let pairs: Vec<(&str, &str)> = [
        ("model", labels.model),
        ("api_provider", labels.api_provider),
        ("user", labels.user),
        ("user_email", labels.user_email),
        ("api_key_alias", labels.api_key_alias),
    ]
    .into_iter()
    .map(|(name, value)| (name, value.unwrap_or(MISSING_LABEL_VALUE)))
    .collect();
    if let Err(error) = metrics.increment_fixed(DeploymentMetric::ManagedBatchCreated, &pairs) {
        tracing::warn!("Error recording batch created metric: {error}");
    }
}

impl CallbackFailureCounter for PrometheusCollectors {
    fn increment_callback_logging_failure(&self, callback_name: &str) {
        // ast-grep-ignore: no-long-names
        let counter = &self.litellm_callback_logging_failures_metric;
        increment_callback_logging_failure(counter, callback_name);
    }
}
