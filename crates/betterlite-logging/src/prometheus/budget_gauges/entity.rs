use betterlite_prometheus::collector_factory::collector::{Collector, MetricValue};
use betterlite_prometheus::label_value_record::UserApiKeyLabelValues;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;

use crate::prometheus_helpers::bounded_series_tracker::RemovableMetric;
use crate::prometheus_label_projection::prometheus_label_factory::{
    LabelSource, prometheus_label_factory,
};

use super::BudgetGaugeError;
use super::gauge::{EntityBudgetGauges, label_pairs};
use super::remaining_budget::remaining_budget;
use super::reset_hours::hours_until_reset;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BudgetGaugeAmounts {
    pub max_budget: Option<Decimal>,
    pub spend: Option<Decimal>,
    pub reset_at: Option<DateTime<Utc>>,
}

// team, org and user rebuild every gauge's labels from that gauge's own list, so the three setters share this walk
pub(crate) fn set_entity_budget_gauges<Gauge>(
    gauges: &EntityBudgetGauges<Gauge>,
    values: &UserApiKeyLabelValues,
    amounts: BudgetGaugeAmounts,
    source: LabelSource<'_>,
    now: DateTime<Utc>,
) -> Result<(), BudgetGaugeError<Gauge::Error>>
where
    Gauge: Collector + RemovableMetric,
{
    let remaining = prometheus_label_factory(&gauges.remaining.supported_labels, values, source)?;
    gauges
        .remaining
        .collector
        .labels(&label_pairs(&remaining))
        .set(remaining_budget(amounts.max_budget, amounts.spend).metric_value());
    if let Some(max_budget) = amounts.max_budget {
        let max = prometheus_label_factory(&gauges.max.supported_labels, values, source)?;
        gauges
            .max
            .collector
            .labels(&label_pairs(&max))
            .set(MetricValue::Finite(max_budget));
    }
    if let Some(reset_at) = amounts.reset_at {
        let hours_labels =
            prometheus_label_factory(&gauges.remaining_hours.supported_labels, values, source)?;
        let hours = hours_until_reset(reset_at, now);
        let value = Decimal::from_f64(hours).ok_or(BudgetGaugeError::HoursNotFinite { hours })?;
        gauges
            .remaining_hours
            .collector
            .labels(&label_pairs(&hours_labels))
            .set(MetricValue::Finite(value));
    }
    Ok(())
}
