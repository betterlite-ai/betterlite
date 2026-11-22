use std::time::Instant;

use betterlite_models::decisions::Json;
use betterlite_prometheus::collector_factory::collector::{Collector as _, MetricValue};
use betterlite_prometheus::label_value_record::UserApiKeyLabelValues;
use rust_decimal::Decimal;

use crate::end_user::EndUserCostTrackingSettings;
use crate::prometheus::bounded_series::CounterEmission;
use crate::prometheus::budget_gauges::gauge::label_pairs;
use crate::prometheus::failure::RequestedModelCatalog;
use crate::prometheus::failure::deployment_labels::{
    DeploymentFailureLabels, DeploymentFailureSources, deployment_failure_labels,
};
use crate::prometheus::failure::exception::LabelException;
use crate::prometheus::failure::exception_name::exception_class_name;
// ast-grep-ignore: no-long-names
use crate::prometheus::failure::invalid_key::should_skip_metrics_for_invalid_key as skip_invalid_key_metrics;
use crate::prometheus::failure::model_cardinality::bounded_requested_model_label;
use crate::prometheus_helpers::bounded_series_tracker::{RemovableMetric, SeriesLimits};
use crate::prometheus_label_projection::label_factory_context::PrometheusLabelFactoryContext;
use crate::prometheus_label_projection::prometheus_label_factory::{
    LabelSource, ProjectedLabels, prometheus_label_factory,
};
use crate::prometheus_label_schema::PrometheusLabelSettings;
use crate::prometheus_rate_limits::gauge::RateLimitGauges;

use super::error::PrometheusDeploymentError;
use super::metric::DeploymentMetric;
use super::values::check_labels;

pub(super) type DeploymentValueResult<Value, Metrics> = Result<
    Value,
    PrometheusDeploymentError<<<Metrics as RateLimitGauges>::Gauge as RemovableMetric>::Error>,
>;
pub(super) type DeploymentResult<Metrics> = DeploymentValueResult<(), Metrics>;

// the collectors plus the settings, end-user switches and series limits one request writes with
pub trait DeploymentMetrics: RateLimitGauges {
    fn metric(&self, metric: DeploymentMetric) -> Self::Gauge;
    fn labels_for(&self, metric: DeploymentMetric) -> &[String];
    fn settings(&self) -> &PrometheusLabelSettings;
    fn end_user_settings(&self) -> EndUserCostTrackingSettings;
    fn limits(&self) -> &SeriesLimits;

    fn label_source<'metrics>(
        &'metrics self,
        context: Option<&'metrics PrometheusLabelFactoryContext>,
    ) -> LabelSource<'metrics> {
        match context {
            Some(context) => LabelSource::Context(context),
            None => LabelSource::Values {
                settings: self.settings(),
                end_user_settings: self.end_user_settings(),
            },
        }
    }

    fn failure_labels(&self, sources: &DeploymentFailureSources<'_>) -> DeploymentFailureLabels {
        deployment_failure_labels(sources)
    }

    fn exception_class(&self, exception: &LabelException) -> String {
        exception_class_name(exception)
    }

    fn bounded_model_label(
        &self,
        requested_model: Option<&Json>,
        router_originated: bool,
        catalog: Option<&dyn RequestedModelCatalog>,
    ) -> Option<String> {
        bounded_requested_model_label(requested_model, router_originated, catalog)
    }

    fn skip_invalid_key(
        &self,
        enum_status_code: Option<&str>,
        exception: Option<&LabelException>,
        kwargs_exception: Option<&LabelException>,
    ) -> bool {
        skip_invalid_key_metrics(enum_status_code, exception, kwargs_exception)
    }

    fn increment_counter(
        &self,
        metric: DeploymentMetric,
        values: &UserApiKeyLabelValues,
        source: LabelSource<'_>,
        now: Instant,
    ) -> DeploymentResult<Self> {
        emit_labeled_counter(self, metric, values, source, now)
    }

    fn set_gauge(
        &self,
        metric: DeploymentMetric,
        values: &UserApiKeyLabelValues,
        source: LabelSource<'_>,
        value: Decimal,
    ) -> DeploymentResult<Self> {
        let labels = prometheus_label_factory(self.labels_for(metric), values, source)?;
        let pairs = label_pairs(&labels);
        self.metric(metric)
            .labels(&pairs)
            .set(MetricValue::Finite(value));
        Ok(())
    }

    fn observe_histogram(
        &self,
        metric: DeploymentMetric,
        labels: &ProjectedLabels,
        value: Decimal,
    ) {
        let pairs = label_pairs(labels);
        self.metric(metric)
            .labels(&pairs)
            .observe(MetricValue::Finite(value));
    }

    fn increment_plain(&self, metric: DeploymentMetric, pairs: &[(&str, &str)]) {
        self.metric(metric)
            .labels(pairs)
            .inc(MetricValue::Finite(Decimal::ONE));
    }

    fn increment_fixed(
        &self,
        metric: DeploymentMetric,
        pairs: &[(&str, &str)],
    ) -> DeploymentResult<Self> {
        check_labels(metric.name(), self.labels_for(metric), pairs)?;
        self.increment_plain(metric, pairs);
        Ok(())
    }
}

// the one labeled emission, so a metrics type that overrides increment_counter keeps the same series
pub(crate) fn emit_labeled_counter<Metrics>(
    metrics: &Metrics,
    metric: DeploymentMetric,
    values: &UserApiKeyLabelValues,
    source: LabelSource<'_>,
    now: Instant,
) -> DeploymentResult<Metrics>
where
    Metrics: DeploymentMetrics,
{
    let counter = metrics.metric(metric);
    let emission = CounterEmission {
        counter: &counter,
        metric_name: metric.name(),
        supported_labels: metrics.labels_for(metric),
        tracker: metrics.tracker(),
        limits: metrics.limits(),
    };
    emission.inc_labeled_counter(values, source, MetricValue::Finite(Decimal::ONE), now)?;
    Ok(())
}
