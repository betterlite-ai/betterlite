use std::collections::BTreeMap;

use betterlite_logging::prometheus::budget_gauges::gauge::{
    BudgetGauges, EntityBudgetGauges, GaugeField,
};
use betterlite_logging::prometheus_exporter::{Handle, PrometheusCollectors};

type LabelMap = BTreeMap<String, Vec<String>>;
type EntityGauges = EntityBudgetGauges<Handle>;

// B3882's gauges read their labels from the schema B3875 registered, so the budget setters get the same metric names
pub(crate) fn budget_gauges(collectors: &PrometheusCollectors) -> BudgetGauges<Handle> {
    let labels = &collectors.cached_labels;
    BudgetGauges {
        team: team_gauges(collectors, labels),
        org: org_gauges(collectors, labels),
        key: key_gauges(collectors, labels),
        user: user_gauges(collectors, labels),
        customer: customer_gauges(collectors, labels),
        team_members: gauge_field(
            labels,
            &collectors.litellm_team_members_metric,
            "litellm_team_members_metric",
        ),
        provider_remaining: gauge_field(
            labels,
            &collectors.litellm_provider_remaining_budget_metric, // ast-grep-ignore: no-long-names
            "litellm_provider_remaining_budget_metric",
        ),
    }
}

fn team_gauges(collectors: &PrometheusCollectors, labels: &LabelMap) -> EntityGauges {
    entity(
        labels,
        &collectors.litellm_remaining_team_budget_metric, // ast-grep-ignore: no-long-names
        &collectors.litellm_team_max_budget_metric,
        &collectors.litellm_team_budget_remaining_hours_metric, // ast-grep-ignore: no-long-names
        [
            "litellm_remaining_team_budget_metric",
            "litellm_team_max_budget_metric",
            "litellm_team_budget_remaining_hours_metric",
        ],
    )
}

fn org_gauges(collectors: &PrometheusCollectors, labels: &LabelMap) -> EntityGauges {
    entity(
        labels,
        &collectors.litellm_remaining_org_budget_metric, // ast-grep-ignore: no-long-names
        &collectors.litellm_org_max_budget_metric,
        &collectors.litellm_org_budget_remaining_hours_metric, // ast-grep-ignore: no-long-names
        [
            "litellm_remaining_org_budget_metric",
            "litellm_org_max_budget_metric",
            "litellm_org_budget_remaining_hours_metric",
        ],
    )
}

fn key_gauges(collectors: &PrometheusCollectors, labels: &LabelMap) -> EntityGauges {
    entity(
        labels,
        &collectors.litellm_remaining_api_key_budget_metric, // ast-grep-ignore: no-long-names
        &collectors.litellm_api_key_max_budget_metric,       // ast-grep-ignore: no-long-names
        &collectors.litellm_api_key_budget_remaining_hours_metric, // ast-grep-ignore: no-long-names
        [
            "litellm_remaining_api_key_budget_metric",
            "litellm_api_key_max_budget_metric",
            "litellm_api_key_budget_remaining_hours_metric",
        ],
    )
}

fn user_gauges(collectors: &PrometheusCollectors, labels: &LabelMap) -> EntityGauges {
    entity(
        labels,
        &collectors.litellm_remaining_user_budget_metric, // ast-grep-ignore: no-long-names
        &collectors.litellm_user_max_budget_metric,
        &collectors.litellm_user_budget_remaining_hours_metric, // ast-grep-ignore: no-long-names
        [
            "litellm_remaining_user_budget_metric",
            "litellm_user_max_budget_metric",
            "litellm_user_budget_remaining_hours_metric",
        ],
    )
}

fn customer_gauges(collectors: &PrometheusCollectors, labels: &LabelMap) -> EntityGauges {
    entity(
        labels,
        &collectors.litellm_remaining_customer_budget_metric, // ast-grep-ignore: no-long-names
        &collectors.litellm_customer_max_budget_metric,       // ast-grep-ignore: no-long-names
        &collectors.litellm_customer_budget_remaining_hours_metric, // ast-grep-ignore: no-long-names
        [
            "litellm_remaining_customer_budget_metric",
            "litellm_customer_max_budget_metric",
            "litellm_customer_budget_remaining_hours_metric",
        ],
    )
}

fn entity(
    labels: &LabelMap,
    remaining: &Handle,
    max: &Handle,
    remaining_hours: &Handle,
    names: [&'static str; 3],
) -> EntityGauges {
    let [remaining_name, max_name, hours_name] = names;
    EntityBudgetGauges {
        remaining: gauge_field(labels, remaining, remaining_name),
        max: gauge_field(labels, max, max_name),
        remaining_hours: gauge_field(labels, remaining_hours, hours_name),
    }
}

fn gauge_field(
    labels: &LabelMap,
    collector: &Handle,
    metric_name: &'static str,
) -> GaugeField<Handle> {
    GaugeField {
        metric_name,
        supported_labels: labels.get(metric_name).cloned().unwrap_or_default(),
        collector: collector.clone(),
    }
}
