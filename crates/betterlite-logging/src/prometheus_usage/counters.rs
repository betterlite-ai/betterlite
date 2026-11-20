use std::time::Instant;

use betterlite_prometheus::collector_factory::collector::{Collector, MetricValue};
use betterlite_prometheus::label_value_record::UserApiKeyLabelValues;
use rust_decimal::Decimal;

use crate::prometheus::bounded_series::CounterEmission;
use crate::prometheus_helpers::bounded_series_tracker::{
    BoundedPrometheusSeriesTracker, RemovableMetric, SeriesLimits,
};
use crate::prometheus_label_projection::label_factory_context::PrometheusLabelFactoryContext;
use crate::prometheus_label_projection::prometheus_label_factory::{LabelSource, ProjectedLabels};
use crate::prometheus_label_schema::PrometheusLabelSettings;

use super::error::PrometheusUsageError;
use super::metric::UsageMetric;

// prometheus_client renders every label value with str(), and Python renders None as "None"
pub(super) const MISSING_LABEL_VALUE: &str = "None";

#[derive(Clone, Copy)]
pub struct UsageLabels<'labels> {
    pub values: &'labels UserApiKeyLabelValues,
    pub context: &'labels PrometheusLabelFactoryContext,
    pub settings: &'labels PrometheusLabelSettings,
}

pub trait UsageCounters {
    type Counter: Collector + RemovableMetric;

    fn counter(&self, metric: UsageMetric) -> Self::Counter;
    fn supported_labels(&self, metric: UsageMetric) -> &[String];
    fn tracker(&self) -> &BoundedPrometheusSeriesTracker;
    fn limits(&self) -> &SeriesLimits;

    fn inc(
        &self,
        metric: UsageMetric,
        labels: UsageLabels<'_>,
        amount: MetricValue,
        now: Instant,
    ) -> Result<(), PrometheusUsageError<<Self::Counter as RemovableMetric>::Error>> {
        if let MetricValue::Finite(amount) = amount
            && amount < Decimal::ZERO
        {
            return Err(PrometheusUsageError::NegativeAmount { amount });
        }
        let counter = self.counter(metric);
        let emission = CounterEmission {
            counter: &counter,
            metric_name: metric.name(),
            supported_labels: self.supported_labels(metric),
            tracker: self.tracker(),
            limits: self.limits(),
        };
        emission.inc_labeled_counter(
            labels.values,
            LabelSource::Context(labels.context),
            amount,
            now,
        )?;
        Ok(())
    }
}

pub(super) fn label_pairs(labels: &ProjectedLabels) -> Vec<(&str, &str)> {
    labels
        .iter()
        .map(|(name, value)| {
            (
                name.as_str(),
                value.as_deref().unwrap_or(MISSING_LABEL_VALUE),
            )
        })
        .collect()
}
