use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_core::cost::call_price::policy::error::PolicyError;
use betterlite_core::cost::prepare::BillingTokenCounter;
use betterlite_core::cost::special::policy::{SearchQuery, SpecialPricePolicy};
use betterlite_logging::cost_breakdown::{CostBreakdownInput, CostBreakdownTarget};
use betterlite_models::decisions::Json;
use betterlite_models::token_cost::GenericCostResult;
use betterlite_tokenizer::count::error::CountError;
use rust_decimal::Decimal;

pub(super) struct Counter;

#[async_trait]
impl BillingTokenCounter for Counter {
    async fn count_messages(&self, _model: &str, _messages: &[Json]) -> Result<usize, CountError> {
        Ok(0)
    }

    async fn count_text(&self, _model: &str, _text: &str) -> Result<usize, CountError> {
        Ok(0)
    }
}

// The lookup adapter the token stage asks for; a case only prices a search it holds a rate for.
pub(super) struct SearchPricing<'rates> {
    pub(super) rates: &'rates BTreeMap<String, Decimal>,
}

impl SpecialPricePolicy for SearchPricing<'_> {
    fn search_cost(&self, query: &SearchQuery<'_>) -> Result<GenericCostResult, PolicyError> {
        let Some(rate) = self.rates.get(query.model) else {
            return Err(PolicyError::UnavailableAdapter(
                query.provider.unwrap_or("search").to_owned(),
            ));
        };
        let queries = Decimal::from(query.number_of_queries);
        let Some(prompt_cost) = rate.checked_mul(queries) else {
            return Err(PolicyError::UnavailableAdapter(
                query.provider.unwrap_or("search").to_owned(),
            ));
        };
        Ok(GenericCostResult {
            prompt_cost,
            completion_cost: Decimal::ZERO,
        })
    }
}

// The isolated breakdown receiver: a refusing one proves a setter failure never replaces the charge.
pub(super) struct Breakdown {
    pub(super) refuse: bool,
}

impl CostBreakdownTarget for Breakdown {
    type Failure = &'static str;

    fn set_cost_breakdown(&mut self, _input: &CostBreakdownInput) -> Result<(), &'static str> {
        if self.refuse {
            return Err("the receiver refuses the breakdown");
        }
        Ok(())
    }
}

pub(super) fn failure(class: &str, message: String) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
