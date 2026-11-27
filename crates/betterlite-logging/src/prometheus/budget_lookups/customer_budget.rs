use betterlite_prometheus::collector_factory::collector::{Collector, MetricLike};
use rust_decimal::Decimal;

use crate::end_user::EndUserCostTrackingSettings;
use crate::prometheus::budget_gauges::BudgetGaugeError;
use crate::prometheus::budget_gauges::customer::{
    CustomerGaugeContext, set_customer_budget_metrics,
};
use crate::prometheus::budget_gauges::customer_gate::{
    budget_gauges_are_noop, customer_budget_metrics_enabled,
};
use crate::prometheus::budget_gauges::gauge::EntityBudgetGauges;
use crate::prometheus::budget_gauges::inputs::CustomerBudgetGaugeInput;
use crate::prometheus_helpers::bounded_series_tracker::RemovableMetric;

use super::lookups::BudgetLookups;

pub struct CustomerBudgetRequest<'request> {
    pub end_user_id: Option<&'request str>,
    pub response_cost: Decimal,
}

// LiteLLM's _set_customer_budget_metrics_after_api_request: the config gate and the no-op gauges come first, the
// cache is the only source, and a cache miss emits nothing
pub async fn set_customer_budget_metrics_after_api_request<Gauge>(
    lookups: &dyn BudgetLookups,
    gauges: &EntityBudgetGauges<MetricLike<Gauge>>,
    context: CustomerGaugeContext<'_>,
    settings: EndUserCostTrackingSettings,
    request: &CustomerBudgetRequest<'_>,
) -> Result<(), BudgetGaugeError<<MetricLike<Gauge> as RemovableMetric>::Error>>
where
    MetricLike<Gauge>: Collector + RemovableMetric + Sync,
{
    if budget_gauges_are_noop(gauges) || !customer_budget_metrics_enabled(&settings) {
        return Ok(());
    }
    let Some(end_user_id) = request.end_user_id else {
        return Ok(());
    };
    let customer = match lookups.cached_customer(end_user_id).await {
        Ok(Some(customer)) => customer,
        Ok(None) => return Ok(()),
        Err(error) => {
            tracing::debug!("[Non-Blocking] Prometheus: Error getting customer info: {error}");
            return Ok(());
        }
    };
    let budget_table = customer.litellm_budget_table.as_ref();
    let input = CustomerBudgetGaugeInput {
        end_user_id: end_user_id.to_owned(),
        spend: customer.spend.saturating_add(request.response_cost),
        max_budget: budget_table.and_then(|table| table.max_budget),
        budget_reset_at: None,
    };
    set_customer_budget_metrics(gauges, &input, context)
}

mod tests;
