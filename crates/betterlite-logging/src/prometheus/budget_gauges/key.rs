use betterlite_prometheus::collector_factory::collector::{Collector, MetricValue};
use betterlite_prometheus::label_names_catalog::UserApiKeyLabelName;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;

use crate::prometheus_helpers::bounded_series_tracker::RemovableMetric;
use crate::prometheus_label_projection::prometheus_label_factory::{
    LabelSource, ProjectedLabels, prometheus_label_factory,
};

use super::BudgetGaugeError;
use super::gauge::{EntityBudgetGauges, label_pairs, label_values};
use super::inputs::KeyBudgetGaugeInput;
use super::remaining_budget::remaining_budget;
use super::reset_hours::hours_until_reset;

// LiteLLM's _set_key_budget_metrics keeps the last label build, so a user can give the max and hours metrics different lists
pub fn set_key_budget_metrics<Gauge>(
    gauges: &EntityBudgetGauges<Gauge>,
    input: &KeyBudgetGaugeInput,
    source: LabelSource<'_>,
    now: DateTime<Utc>,
) -> Result<(), BudgetGaugeError<Gauge::Error>>
where
    Gauge: Collector + RemovableMetric,
{
    let values = label_values(&[
        (
            UserApiKeyLabelName::HashedApiKey.as_str(),
            input.token.as_str(),
        ),
        (
            UserApiKeyLabelName::ApiKeyAlias.as_str(),
            input.key_alias.as_deref().unwrap_or_default(),
        ),
    ]);
    let remaining = prometheus_label_factory(&gauges.remaining.supported_labels, &values, source)?;
    gauges
        .remaining
        .collector
        .labels(&label_pairs(&remaining))
        .set(remaining_budget(input.max_budget, input.spend).metric_value());
    let mut last_labels = remaining;
    if let Some(max_budget) = input.max_budget {
        let max = prometheus_label_factory(&gauges.max.supported_labels, &values, source)?;
        gauges
            .max
            .collector
            .labels(&label_pairs(&max))
            .set(MetricValue::Finite(max_budget));
        last_labels = max;
    }
    if let Some(reset_at) = input.budget_reset_at {
        set_key_hours_gauge(gauges, &last_labels, reset_at, now)?;
    }
    Ok(())
}

fn set_key_hours_gauge<Gauge>(
    gauges: &EntityBudgetGauges<Gauge>,
    labels: &ProjectedLabels,
    reset_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<(), BudgetGaugeError<Gauge::Error>>
where
    Gauge: Collector + RemovableMetric,
{
    let hours = hours_until_reset(reset_at, now);
    let value = Decimal::from_f64(hours).ok_or(BudgetGaugeError::HoursNotFinite { hours })?;
    gauges
        .remaining_hours
        .collector
        .labels(&label_pairs(labels))
        .set(MetricValue::Finite(value));
    Ok(())
}

mod tests;
