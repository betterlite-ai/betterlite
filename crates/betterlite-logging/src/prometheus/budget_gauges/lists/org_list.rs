use betterlite_prometheus::collector_factory::collector::Collector;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::prometheus_helpers::bounded_series_tracker::RemovableMetric;
use crate::prometheus_label_projection::prometheus_label_factory::LabelSource;

use super::super::BudgetGaugeError;
use super::super::gauge::EntityBudgetGauges;
use super::super::inputs::OrgBudgetGaugeInput;
use super::super::org::set_org_budget_metrics;
use super::BudgetTableSnapshot;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrgBudgetGaugeRow {
    pub organization_id: Option<String>,
    pub organization_alias: Option<String>,
    pub spend: Option<Decimal>,
    pub budget: Option<BudgetTableSnapshot>,
}

pub fn set_org_list_budget_metrics<Gauge>(
    gauges: &EntityBudgetGauges<Gauge>,
    orgs: &[OrgBudgetGaugeRow],
    source: LabelSource<'_>,
    now: DateTime<Utc>,
) -> Result<(), BudgetGaugeError<Gauge::Error>>
where
    Gauge: Collector + RemovableMetric,
{
    for org in orgs {
        let budget = org.budget.as_ref();
        let input = OrgBudgetGaugeInput {
            org_id: org.organization_id.clone().unwrap_or_default(),
            org_alias: org.organization_alias.clone().unwrap_or_default(),
            spend: org.spend.unwrap_or_default(),
            max_budget: budget.and_then(|table| table.max_budget),
            budget_reset_at: budget.and_then(|table| table.budget_reset_at),
        };
        set_org_budget_metrics(gauges, &input, source, now)?;
    }
    Ok(())
}
