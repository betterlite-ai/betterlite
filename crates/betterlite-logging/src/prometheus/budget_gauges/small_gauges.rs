use betterlite_prometheus::collector_factory::collector::{Collector, MetricValue};
use betterlite_prometheus::label_names_catalog::UserApiKeyLabelName;
use rust_decimal::Decimal;

use crate::prometheus_label_projection::prometheus_label_factory::{
    LabelFactoryError, LabelSource, prometheus_label_factory,
};

use super::gauge::{GaugeField, label_pairs, label_values};
use super::inputs::TeamMembersGaugeInput;
use super::remaining_budget::remaining_budget;

pub fn set_team_members_metric<Gauge>(
    field: &GaugeField<Gauge>,
    input: &TeamMembersGaugeInput,
    source: LabelSource<'_>,
) -> Result<(), LabelFactoryError>
where
    Gauge: Collector,
{
    let values = label_values(&[
        (UserApiKeyLabelName::Team.as_str(), input.team_id.as_str()),
        (
            UserApiKeyLabelName::TeamAlias.as_str(),
            input.team_alias.as_deref().unwrap_or_default(),
        ),
    ]);
    let labels = prometheus_label_factory(&field.supported_labels, &values, source)?;
    field
        .collector
        .labels(&label_pairs(&labels))
        .set(MetricValue::Finite(Decimal::from(input.member_count)));
    Ok(())
}

// LiteLLM passes the provider as the only positional label value, so no label factory or include/exclude filter runs
pub fn track_provider_remaining_budget<Gauge>(
    field: &GaugeField<Gauge>,
    provider: &str,
    spend: Option<Decimal>,
    budget_limit: Option<Decimal>,
) where
    Gauge: Collector,
{
    field
        .collector
        .labels(&[(UserApiKeyLabelName::ApiProvider.as_str(), provider)])
        .set(remaining_budget(budget_limit, spend).metric_value());
}

mod tests;
