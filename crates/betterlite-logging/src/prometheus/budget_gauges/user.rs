use betterlite_prometheus::collector_factory::collector::Collector;
use betterlite_prometheus::label_names_catalog::UserApiKeyLabelName;
use chrono::{DateTime, Utc};

use crate::prometheus_helpers::bounded_series_tracker::RemovableMetric;
use crate::prometheus_label_projection::prometheus_label_factory::LabelSource;

use super::BudgetGaugeError;
use super::entity::{BudgetGaugeAmounts, set_entity_budget_gauges};
use super::gauge::{EntityBudgetGauges, label_values};
use super::inputs::UserBudgetGaugeInput;

pub fn set_user_budget_metrics<Gauge>(
    gauges: &EntityBudgetGauges<Gauge>,
    input: &UserBudgetGaugeInput,
    source: LabelSource<'_>,
    now: DateTime<Utc>,
) -> Result<(), BudgetGaugeError<Gauge::Error>>
where
    Gauge: Collector + RemovableMetric,
{
    let values = label_values(&[
        (UserApiKeyLabelName::User.as_str(), input.user_id.as_str()),
        (
            UserApiKeyLabelName::UserEmail.as_str(),
            input.user_email.as_deref().unwrap_or_default(),
        ),
        (
            UserApiKeyLabelName::UserAlias.as_str(),
            input.user_alias.as_deref().unwrap_or_default(),
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
