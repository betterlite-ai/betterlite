use std::collections::BTreeMap;
use std::convert::Infallible;

use betterlite_logging::cost_breakdown::{CostBreakdownInput, CostBreakdownTarget};
use betterlite_models::billed_tokens::BilledTokenRates;
use betterlite_models::cost_breakdown::CostBreakdown;
use rust_decimal::Decimal;

mod tests;

// The breakdown one charge leaves for Python. betterlite-logging's LoggingState stores it too, but it
// needs the whole logging state to exist, so the two field rules below repeat that one impl.
#[derive(Debug, Default)]
pub(crate) struct Capture {
    pub(crate) breakdown: Option<CostBreakdown>,
    pub(crate) rates: Option<BilledTokenRates>,
}

impl CostBreakdownTarget for Capture {
    type Failure = Infallible;

    fn set_cost_breakdown(&mut self, input: &CostBreakdownInput) -> Result<(), Infallible> {
        self.rates = input.billed_token_rates;
        self.breakdown = Some(record(input));
        Ok(())
    }
}

fn record(input: &CostBreakdownInput) -> CostBreakdown {
    CostBreakdown {
        service_tier: input.service_tier.clone(),
        data_residency: input.data_residency.clone(),
        vertex_location: input.vertex_location.clone(),
        input_cost: Some(input.input_cost),
        cache_read_cost: input.cache_read_cost.filter(|cost| *cost > Decimal::ZERO),
        cache_creation_cost: input
            .cache_creation_cost
            .filter(|cost| *cost > Decimal::ZERO),
        output_cost: Some(input.output_cost),
        reasoning_cost: input.reasoning_cost.filter(|cost| *cost > Decimal::ZERO),
        total_cost: Some(input.total_cost),
        tool_usage_cost: Some(input.tool_usage_cost),
        guardrail_cost: None,
        additional_costs: input
            .additional_costs
            .clone()
            .filter(|costs| !costs.is_empty()),
        original_cost: input.original_cost,
        discount_percent: input.discount_percent,
        discount_amount: input.discount_amount,
        margin_percent: input.margin_percent,
        margin_fixed_amount: input.margin_fixed_amount,
        margin_total_amount: input.margin_total_amount,
        extra: BTreeMap::new(),
    }
}
