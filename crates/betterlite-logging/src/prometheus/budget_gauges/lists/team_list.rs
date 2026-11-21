use betterlite_prometheus::collector_factory::collector::Collector;
use chrono::{DateTime, Utc};

use crate::prometheus_helpers::bounded_series_tracker::RemovableMetric;
use crate::prometheus_label_projection::prometheus_label_factory::LabelSource;

use super::super::BudgetGaugeError;
use super::super::gauge::EntityBudgetGauges;
use super::super::inputs::TeamBudgetGaugeInput;
use super::super::team::set_team_budget_metrics;

pub fn set_team_list_budget_metrics<Gauge>(
    gauges: &EntityBudgetGauges<Gauge>,
    teams: &[TeamBudgetGaugeInput],
    source: LabelSource<'_>,
    now: DateTime<Utc>,
) -> Result<(), BudgetGaugeError<Gauge::Error>>
where
    Gauge: Collector + RemovableMetric,
{
    for team in teams {
        set_team_budget_metrics(gauges, team, source, now)?;
    }
    Ok(())
}
