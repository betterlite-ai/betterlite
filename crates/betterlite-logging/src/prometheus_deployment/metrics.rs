use std::time::Instant;

use betterlite_models::decisions::Json;
use betterlite_prometheus::collector_factory::collector::{Collector as _, MetricValue};
use betterlite_prometheus::label_value_record::UserApiKeyLabelValues;
use rust_decimal::Decimal;

use crate::end_user::EndUserCostTrackingSettings;
use crate::prometheus::RequestedModelCatalog;
use crate::prometheus::bounded_series::CounterEmission;
use crate::prometheus::budget_gauges::gauge::label_pairs;
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
use crate::prometheus_latency::guardrail_overhead::{
    OverheadObservation, set_guardrail_overhead_metric,
};
use crate::prometheus_latency::labels::LabelInputs;
use crate::prometheus_latency::latency_histograms::MetricHandle;
use crate::prometheus_rate_limits::deployment::DeploymentLimitInputs;
// ast-grep-ignore: no-long-names
use crate::prometheus_rate_limits::deployment::set_deployment_tpm_rpm_limit_metrics as set_deployment_limit_metrics;
use crate::prometheus_rate_limits::gauge::RateLimitGauges;

use super::error::PrometheusDeploymentError;
use super::metric::DeploymentMetric;
use super::values::check_labels;

// the collectors plus the settings, end-user switches and series limits one request writes with
pub trait DeploymentMetrics: RateLimitGauges {
    fn metric(&self, metric: DeploymentMetric) -> Self::Gauge;
    fn labels_for(&self, metric: DeploymentMetric) -> &[String];
    fn settings(&self) -> &PrometheusLabelSettings;
    fn end_user_settings(&self) -> EndUserCostTrackingSettings;
    fn limits(&self) -> &SeriesLimits;

    fn values_source(&self) -> LabelSource<'_> {
        LabelSource::Values {
            settings: self.settings(),
            end_user_settings: self.end_user_settings(),
        }
    }

    fn label_source<'metrics>(
        &'metrics self,
        context: Option<&'metrics PrometheusLabelFactoryContext>,
    ) -> LabelSource<'metrics> {
        match context {
            Some(context) => LabelSource::Context(context),
            None => self.values_source(),
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

    // the two call sites that hold a group name rather than a payload value
    fn bounded_model_name(
        &self,
        model: Option<&str>,
        catalog: Option<&dyn RequestedModelCatalog>,
    ) -> Option<String> {
        match model {
            Some(name) => {
                let requested = Json::Text(name.to_owned());
                self.bounded_model_label(Some(&requested), true, catalog)
            }
            None => self.bounded_model_label(None, true, catalog),
        }
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
    ) -> Result<(), PrometheusDeploymentError<<Self::Gauge as RemovableMetric>::Error>> {
        let counter = self.metric(metric);
        let limits = self.limits();
        let emission = CounterEmission {
            counter: &counter,
            metric_name: metric.name(),
            supported_labels: self.labels_for(metric),
            tracker: self.tracker(),
            limits,
        };
        emission.inc_labeled_counter(values, source, MetricValue::Finite(Decimal::ONE), now)?;
        Ok(())
    }

    fn set_gauge(
        &self,
        metric: DeploymentMetric,
        values: &UserApiKeyLabelValues,
        source: LabelSource<'_>,
        value: Decimal,
    ) -> Result<(), PrometheusDeploymentError<<Self::Gauge as RemovableMetric>::Error>> {
        let labels = prometheus_label_factory(self.labels_for(metric), values, source)?;
        let gauge = self.metric(metric);
        gauge
            .labels(&label_pairs(&labels))
            .set(MetricValue::Finite(value));
        Ok(())
    }

    fn observe_histogram(
        &self,
        metric: DeploymentMetric,
        labels: &ProjectedLabels,
        value: Decimal,
    ) {
        let histogram = self.metric(metric);
        histogram
            .labels(&label_pairs(labels))
            .observe(MetricValue::Finite(value));
    }

    fn increment_plain(&self, metric: DeploymentMetric, pairs: &[(&str, &str)]) {
        let counter = self.metric(metric);
        counter.labels(pairs).inc(MetricValue::Finite(Decimal::ONE));
    }

    fn increment_fixed(
        &self,
        metric: DeploymentMetric,
        pairs: &[(&str, &str)],
    ) -> Result<(), PrometheusDeploymentError<<Self::Gauge as RemovableMetric>::Error>> {
        check_labels(metric.name(), self.labels_for(metric), pairs)?;
        self.increment_plain(metric, pairs);
        Ok(())
    }

    fn set_deployment_limits(
        &self,
        inputs: DeploymentLimitInputs<'_>,
    ) -> Result<(), PrometheusDeploymentError<<Self::Gauge as RemovableMetric>::Error>> {
        set_deployment_limit_metrics(self, inputs)?;
        Ok(())
    }

    fn set_overhead(
        &self,
        observation: &OverheadObservation<'_>,
        labels: &LabelInputs<'_>,
    ) -> Result<(), PrometheusDeploymentError<<Self::Gauge as RemovableMetric>::Error>> {
        let histogram = self.metric(DeploymentMetric::OverheadWithGuardrails);
        let handle = MetricHandle {
            metric: &histogram,
            supported_labels: self.labels_for(DeploymentMetric::OverheadWithGuardrails),
        };
        set_guardrail_overhead_metric(&handle, observation, labels)?;
        Ok(())
    }
}
