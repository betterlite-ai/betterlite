use std::time::Instant;

use betterlite_prometheus::collector_factory::collector::{Collector, MetricValue};
use betterlite_prometheus::label_names_catalog::UserApiKeyLabelName;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;

use crate::prometheus::bounded_series::CounterEmission;
use crate::prometheus_helpers::bounded_series_tracker::{
    BoundedPrometheusSeriesTracker, RemovableMetric, SeriesLimits,
};
use crate::prometheus_label_projection::prometheus_label_factory::{
    LabelSource, ProjectedLabels, prometheus_label_factory,
};

use super::BudgetGaugeError;
use super::gauge::{EntityBudgetGauges, GaugeField, label_pairs, label_values};
use super::inputs::CustomerBudgetGaugeInput;
use super::remaining_budget::remaining_budget;
use super::reset_hours::hours_until_reset;

// the hours gauge reads the wall clock while the series tracker reads the monotonic one, so both come in together
#[derive(Clone, Copy, Debug)]
pub struct BudgetGaugeClock {
    pub reset: DateTime<Utc>,
    pub monotonic: Instant,
}

#[derive(Clone, Copy)]
pub struct CustomerGaugeContext<'context> {
    pub source: LabelSource<'context>,
    pub clock: BudgetGaugeClock,
    pub tracker: &'context BoundedPrometheusSeriesTracker,
    pub limits: &'context SeriesLimits,
}

pub fn set_customer_budget_metrics<Gauge>(
    gauges: &EntityBudgetGauges<Gauge>,
    input: &CustomerBudgetGaugeInput,
    context: CustomerGaugeContext<'_>,
) -> Result<(), BudgetGaugeError<Gauge::Error>>
where
    Gauge: Collector + RemovableMetric,
{
    let values = label_values(&[(
        UserApiKeyLabelName::EndUser.as_str(),
        input.end_user_id.as_str(),
    )]);
    let labels =
        prometheus_label_factory(&gauges.remaining.supported_labels, &values, context.source)?;
    if labels
        .get(UserApiKeyLabelName::EndUser.as_str())
        .is_none_or(Option::is_none)
    {
        return Ok(());
    }
    gauges
        .remaining
        .collector
        .labels(&label_pairs(&labels))
        .set(remaining_budget(input.max_budget, Some(input.spend)).metric_value());
    track_end_user_series(&gauges.remaining, &labels, context)?;
    if let Some(max_budget) = input.max_budget {
        gauges
            .max
            .collector
            .labels(&label_pairs(&labels))
            .set(MetricValue::Finite(max_budget));
        track_end_user_series(&gauges.max, &labels, context)?;
    }
    if input.budget_reset_at.is_some() {
        set_customer_hours_gauge(gauges, input, &labels, context)?;
    }
    Ok(())
}

fn set_customer_hours_gauge<Gauge>(
    gauges: &EntityBudgetGauges<Gauge>,
    input: &CustomerBudgetGaugeInput,
    labels: &ProjectedLabels,
    context: CustomerGaugeContext<'_>,
) -> Result<(), BudgetGaugeError<Gauge::Error>>
where
    Gauge: Collector + RemovableMetric,
{
    let Some(reset_at) = input.budget_reset_at else {
        return Ok(());
    };
    let hours = hours_until_reset(reset_at, context.clock.reset);
    let value = Decimal::from_f64(hours).ok_or(BudgetGaugeError::HoursNotFinite { hours })?;
    gauges
        .remaining_hours
        .collector
        .labels(&label_pairs(labels))
        .set(MetricValue::Finite(value));
    track_end_user_series(&gauges.remaining_hours, labels, context)
}

fn track_end_user_series<Gauge>(
    field: &GaugeField<Gauge>,
    labels: &ProjectedLabels,
    context: CustomerGaugeContext<'_>,
) -> Result<(), BudgetGaugeError<Gauge::Error>>
where
    Gauge: Collector + RemovableMetric,
{
    let emission = CounterEmission {
        counter: &field.collector,
        metric_name: field.metric_name,
        supported_labels: &field.supported_labels,
        tracker: context.tracker,
        limits: context.limits,
    };
    emission.track_end_user(labels, context.clock.monotonic)?;
    Ok(())
}

mod tests;
