use std::collections::BTreeMap;
use std::convert::Infallible;
use std::fmt::Debug;

use betterlite_models::billed_tokens::BilledTokenRates;
use betterlite_models::cost_breakdown::CostBreakdown;
use rust_decimal::Decimal;

use crate::logging_state::LoggingState;

mod tests;

pub struct CostBreakdownInput {
    pub input_cost: Decimal,
    pub output_cost: Decimal,
    pub total_cost: Decimal,
    pub tool_usage_cost: Decimal,
    pub additional_costs: Option<BTreeMap<String, Decimal>>,
    pub original_cost: Option<Decimal>,
    pub discount_percent: Option<Decimal>,
    pub discount_amount: Option<Decimal>,
    pub margin_percent: Option<Decimal>,
    pub margin_fixed_amount: Option<Decimal>,
    pub margin_total_amount: Option<Decimal>,
    pub cache_read_cost: Option<Decimal>,
    pub cache_creation_cost: Option<Decimal>,
    pub reasoning_cost: Option<Decimal>,
    pub service_tier: Option<String>,
    pub data_residency: Option<String>,
    pub vertex_location: Option<String>,
    pub billed_token_rates: Option<BilledTokenRates>,
}

// Assigning the logging object's own record cannot fail; a caller's other target may, and the cost caller only debug-logs that.
pub trait CostBreakdownTarget {
    type Failure: Debug;

    fn set_cost_breakdown(&mut self, input: &CostBreakdownInput) -> Result<(), Self::Failure>;
}

impl CostBreakdownTarget for LoggingState {
    type Failure = Infallible;

    fn set_cost_breakdown(&mut self, input: &CostBreakdownInput) -> Result<(), Self::Failure> {
        self.billed_token_rates = input.billed_token_rates;
        self.cost_breakdown = Some(CostBreakdown {
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
        });
        Ok(())
    }
}
