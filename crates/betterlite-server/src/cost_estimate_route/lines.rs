use std::convert::Infallible;

use betterlite_logging::cost_breakdown::{CostBreakdownInput, CostBreakdownTarget};
use betterlite_models::billed_tokens::BilledTokenRates;
use rust_decimal::Decimal;

// The logging object's own breakdown fields the endpoint reads back.
#[derive(Default)]
pub(super) struct Capture {
    input: Option<Decimal>,
    output: Option<Decimal>,
    margin: Option<Decimal>,
    cache_read: Option<Decimal>,
    cache_creation: Option<Decimal>,
    reasoning: Option<Decimal>,
    rates: Option<BilledTokenRates>,
}

impl Capture {
    pub(super) fn finish(self, total: Decimal) -> (CostLines, Option<BilledTokenRates>) {
        let lines = CostLines {
            total,
            input: self.input.unwrap_or_default(),
            output: self.output.unwrap_or_default(),
            margin: self.margin.unwrap_or_default(),
            cache_read: self.cache_read.unwrap_or_default(),
            cache_creation: self.cache_creation.unwrap_or_default(),
            reasoning: self.reasoning.unwrap_or_default(),
        };
        (lines, self.rates)
    }
}

impl CostBreakdownTarget for Capture {
    type Failure = Infallible;

    fn set_cost_breakdown(&mut self, input: &CostBreakdownInput) -> Result<(), Infallible> {
        self.input = Some(input.input_cost);
        self.output = Some(input.output_cost);
        self.margin = input.margin_total_amount;
        self.cache_read = input.cache_read_cost;
        self.cache_creation = input.cache_creation_cost;
        self.reasoning = input.reasoning_cost;
        self.rates = input.billed_token_rates;
        Ok(())
    }
}

pub(super) struct CostLines {
    pub(super) total: Decimal,
    pub(super) input: Decimal,
    pub(super) output: Decimal,
    pub(super) margin: Decimal,
    pub(super) cache_read: Decimal,
    pub(super) cache_creation: Decimal,
    pub(super) reasoning: Decimal,
}

impl CostLines {
    // CostLines.times: a falsy count answers none, a present one scales every line.
    pub(super) fn times(&self, count: Option<i64>) -> Option<Self> {
        let factor = Decimal::from(count.filter(|count| *count != 0)?);
        Some(Self {
            total: self.total.saturating_mul(factor),
            input: self.input.saturating_mul(factor),
            output: self.output.saturating_mul(factor),
            margin: self.margin.saturating_mul(factor),
            cache_read: self.cache_read.saturating_mul(factor),
            cache_creation: self.cache_creation.saturating_mul(factor),
            reasoning: self.reasoning.saturating_mul(factor),
        })
    }
}
