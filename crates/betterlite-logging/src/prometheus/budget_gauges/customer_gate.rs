use betterlite_prometheus::collector_factory::collector::MetricLike;

use crate::end_user::EndUserCostTrackingSettings;

use super::gauge::EntityBudgetGauges;

// LiteLLM's _customer_budget_metrics_enabled: an exact-true opt-in, and the disable flag must not be set
#[must_use]
// ast-grep-ignore: no-long-names
pub const fn customer_budget_metrics_enabled(settings: &EndUserCostTrackingSettings) -> bool {
    matches!(settings.enable_prometheus_only, Some(true))
        && !matches!(settings.disable_end_user_cost_tracking, Some(true))
}

// LiteLLM's _customer_budget_gauges_are_noop: with every customer gauge disabled the per-request path can skip its work
#[must_use]
pub fn budget_gauges_are_noop<Gauge>(gauges: &EntityBudgetGauges<MetricLike<Gauge>>) -> bool {
    let fields = [&gauges.remaining, &gauges.max, &gauges.remaining_hours];
    fields
        .iter()
        .all(|field| matches!(&field.collector, MetricLike::NoOp(_)))
}

mod tests;
