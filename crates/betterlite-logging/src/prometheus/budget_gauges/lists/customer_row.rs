use betterlite_prometheus::collector_factory::collector::Collector;
use rust_decimal::Decimal;

use crate::prometheus_helpers::bounded_series_tracker::RemovableMetric;

use super::super::BudgetGaugeError;
use super::super::customer::{CustomerGaugeContext, set_customer_budget_metrics};
use super::super::gauge::EntityBudgetGauges;
use super::super::inputs::CustomerBudgetGaugeInput;
use super::BudgetTableSnapshot;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CustomerBudgetGaugeRow {
    pub user_id: String,
    pub spend: Decimal,
    pub budget: Option<BudgetTableSnapshot>,
}

// the one customer path that passes a reset time: the refresh reads it from the row's budget table
pub fn set_customer_row_gauges<Gauge>(
    gauges: &EntityBudgetGauges<Gauge>,
    customer: &CustomerBudgetGaugeRow,
    default_budget: Option<&BudgetTableSnapshot>,
    context: CustomerGaugeContext<'_>,
) -> Result<(), BudgetGaugeError<Gauge::Error>>
where
    Gauge: Collector + RemovableMetric,
{
    let budget = customer.budget.as_ref().or(default_budget);
    let input = CustomerBudgetGaugeInput {
        end_user_id: customer.user_id.clone(),
        spend: customer.spend,
        max_budget: budget.and_then(|table| table.max_budget),
        budget_reset_at: budget.and_then(|table| table.budget_reset_at),
    };
    set_customer_budget_metrics(gauges, &input, context)
}
