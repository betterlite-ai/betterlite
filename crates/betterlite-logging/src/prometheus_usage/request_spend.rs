use std::time::Instant;

use betterlite_models::zero_cost::ZeroCostDiagnostic;
use betterlite_prometheus::collector_factory::collector::MetricValue;
use betterlite_prometheus::label_names_catalog::ZERO_COST_REASON_LABEL;
use rust_decimal::Decimal;

use crate::prometheus_label_projection::prometheus_label_factory::{
    LabelSource, ProjectedLabels, prometheus_label_factory,
};

use super::counters::{MISSING_LABEL_VALUE, UsageCounters, UsageLabels};
use super::error::PrometheusUsageError;
use super::metric::UsageMetric;
use crate::fields::payload::StandardLoggingPayload;

pub fn increment_request_and_spend<C: UsageCounters>(
    counters: &C,
    payload: &StandardLoggingPayload,
    labels: UsageLabels<'_>,
    now: Instant,
) -> Result<(), PrometheusUsageError<C::Counter::Error>> {
    counters.inc(
        UsageMetric::Requests,
        labels,
        MetricValue::Finite(Decimal::ONE),
        now,
    )?;
    counters.inc(
        UsageMetric::Spend,
        labels,
        MetricValue::Finite(payload.response_cost),
        now,
    )
}

pub fn increment_zero_cost_requests<C: UsageCounters>(
    counters: &C,
    diagnostic: Option<&ZeroCostDiagnostic>,
    labels: UsageLabels<'_>,
) -> Result<(), PrometheusUsageError<C::Counter::Error>> {
    let Some(diagnostic) = diagnostic else {
        return Ok(());
    };
    let supported = counters.supported_labels(UsageMetric::ZeroCostRequests);
    let mut projected = prometheus_label_factory(
        supported,
        labels.values,
        LabelSource::Context(labels.context),
    )?;
    if supported
        .iter()
        .any(|label| label.as_str() == ZERO_COST_REASON_LABEL)
    {
        projected.insert(
            ZERO_COST_REASON_LABEL.to_owned(),
            Some(diagnostic.reason.label().to_owned()),
        );
    }
    let pairs = label_pairs(&projected);
    counters
        .counter(UsageMetric::ZeroCostRequests)
        .labels(&pairs)
        .inc(MetricValue::Finite(Decimal::ONE));
    Ok(())
}

fn label_pairs(labels: &ProjectedLabels) -> Vec<(&str, &str)> {
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
