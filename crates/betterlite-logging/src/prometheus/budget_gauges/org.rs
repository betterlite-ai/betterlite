use betterlite_prometheus::collector_factory::collector::Collector;
use betterlite_prometheus::label_names_catalog::UserApiKeyLabelName;
use chrono::{DateTime, Utc};

use crate::prometheus_helpers::bounded_series_tracker::RemovableMetric;
use crate::prometheus_label_projection::prometheus_label_factory::LabelSource;

use super::BudgetGaugeError;
use super::entity::{BudgetGaugeAmounts, set_entity_budget_gauges};
use super::gauge::{EntityBudgetGauges, label_values};
use super::inputs::OrgBudgetGaugeInput;

pub fn set_org_budget_metrics<Gauge>(
    gauges: &EntityBudgetGauges<Gauge>,
    input: &OrgBudgetGaugeInput,
    source: LabelSource<'_>,
    now: DateTime<Utc>,
) -> Result<(), BudgetGaugeError<Gauge::Error>>
where
    Gauge: Collector + RemovableMetric,
{
    let values = label_values(&[
        (UserApiKeyLabelName::OrgId.as_str(), input.org_id.as_str()),
        (
            UserApiKeyLabelName::OrgAlias.as_str(),
            input.org_alias.as_str(),
        ),
    ]);
    let amounts = BudgetGaugeAmounts {
        max_budget: input.max_budget,
        spend: input.spend,
        reset_at: input.budget_reset_at,
    };
    set_entity_budget_gauges(gauges, &values, amounts, source, now)
}

mod tests;
