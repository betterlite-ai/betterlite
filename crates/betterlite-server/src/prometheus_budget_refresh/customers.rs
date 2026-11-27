use async_trait::async_trait;
use betterlite_logging::end_user::EndUserCostTrackingSettings;
use betterlite_logging::prometheus::budget_gauges::customer::CustomerGaugeContext;
use betterlite_logging::prometheus::budget_gauges::customer_gate::{
    budget_gauges_are_noop,
    // ast-grep-ignore: no-long-names
    customer_budget_metrics_enabled as customer_metrics_enabled,
};
use betterlite_logging::prometheus::budget_gauges::gauge::EntityBudgetGauges;
use betterlite_logging::prometheus::budget_gauges::lists::BudgetTableSnapshot;
use betterlite_logging::prometheus::budget_gauges::lists::customer_row::{
    CustomerBudgetGaugeRow, set_customer_row_gauges,
};
use betterlite_logging::prometheus::budget_lookups::customer_default::get_default_customer_budget;
use betterlite_logging::prometheus::budget_lookups::lookups::BudgetLookups;
use betterlite_logging::prometheus_helpers::bounded_series_tracker::RemovableMetric;
use betterlite_prometheus::collector_factory::collector::{Collector, MetricLike};

use super::page_walk::{EntityPageWalk, initialize_entity};
use super::source::{BudgetPageError, BudgetPageSource, Page};

mod tests;

struct CustomerWalk<'walk, Gauge>
where
    MetricLike<Gauge>: Collector + RemovableMetric,
{
    source: &'walk dyn BudgetPageSource,
    gauges: &'walk EntityBudgetGauges<MetricLike<Gauge>>,
    default_budget: Option<&'walk BudgetTableSnapshot>,
    budgeted_only: bool,
    context: CustomerGaugeContext<'walk>,
}

#[async_trait]
impl<Gauge> EntityPageWalk for CustomerWalk<'_, Gauge>
where
    MetricLike<Gauge>: Collector + RemovableMetric + Send + Sync,
    <MetricLike<Gauge> as RemovableMetric>::Error: Send + Sync,
{
    type Row = CustomerBudgetGaugeRow;

    async fn fetch(&self, page_size: u64, page: u64) -> Result<Page<Self::Row>, BudgetPageError> {
        self.source
            .customers_page(page_size, page, self.budgeted_only)
            .await
    }

    fn apply(&self, rows: Vec<Self::Row>) -> Result<(), BudgetPageError> {
        for row in &rows {
            set_customer_row_gauges(self.gauges, row, self.default_budget, self.context).map_err(
                |error| BudgetPageError::Gauge {
                    source: Box::new(error),
                },
            )?;
        }
        Ok(())
    }
}

pub struct CustomerRefresh<'refresh, Gauge>
where
    MetricLike<Gauge>: Collector + RemovableMetric,
{
    pub source: Option<&'refresh dyn BudgetPageSource>,
    pub lookups: &'refresh dyn BudgetLookups,
    pub default_budget_id: Option<&'refresh str>,
    pub gauges: &'refresh EntityBudgetGauges<MetricLike<Gauge>>,
    pub end_user_settings: EndUserCostTrackingSettings,
    pub context: CustomerGaugeContext<'refresh>,
}

// LiteLLM's _initialize_customer_budget_metrics: the database, no-op gauges and enabled gate come before the default budget read
pub async fn refresh_customers<Gauge>(refresh: &CustomerRefresh<'_, Gauge>)
where
    MetricLike<Gauge>: Collector + RemovableMetric + Send + Sync,
    <MetricLike<Gauge> as RemovableMetric>::Error: Send + Sync,
{
    let Some(source) = refresh.source else {
        tracing::debug!("Prometheus: skipping customer metrics initialization, DB not initialized");
        return;
    };
    if budget_gauges_are_noop(refresh.gauges) {
        return;
    }
    if !customer_metrics_enabled(&refresh.end_user_settings) {
        tracing::debug!(
            "Prometheus: skipping customer metrics initialization, end_user tracking disabled"
        );
        return;
    }
    let default_budget =
        get_default_customer_budget(refresh.lookups, refresh.default_budget_id).await;
    let walk = CustomerWalk {
        source,
        gauges: refresh.gauges,
        default_budget: default_budget.as_ref(),
        budgeted_only: default_budget.is_none(),
        context: refresh.context,
    };
    initialize_entity(&walk, "customers").await;
}
