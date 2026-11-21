use betterlite_prometheus::collector_factory::collector::Collector;
use chrono::{DateTime, Utc};

use crate::prometheus_helpers::bounded_series_tracker::RemovableMetric;
use crate::prometheus_label_projection::prometheus_label_factory::LabelSource;

use super::super::BudgetGaugeError;
use super::super::gauge::EntityBudgetGauges;
use super::super::inputs::UserBudgetGaugeInput;
use super::super::user::set_user_budget_metrics;

pub fn set_user_list_budget_metrics<Gauge>(
    gauges: &EntityBudgetGauges<Gauge>,
    users: &[UserBudgetGaugeInput],
    source: LabelSource<'_>,
    now: DateTime<Utc>,
) -> Result<(), BudgetGaugeError<Gauge::Error>>
where
    Gauge: Collector + RemovableMetric,
{
    for user in users {
        set_user_budget_metrics(gauges, user, source, now)?;
    }
    Ok(())
}
