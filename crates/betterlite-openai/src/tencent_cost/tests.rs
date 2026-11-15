#![cfg(test)]

use std::cell::RefCell;
use std::collections::BTreeMap;

use betterlite_models::cost_usage::CostUsage;
use betterlite_models::token_cost::GenericCostResult;
use betterlite_models::token_cost::error::TokenCostError;
use rust_decimal::Decimal;

use super::{PROVIDER, TencentCostError, cost_per_token};
use crate::xai_usage::GenericTokenPricing;

fn usage() -> CostUsage {
    CostUsage {
        prompt_tokens: 11,
        completion_tokens: 4,
        total_tokens: 15,
        prompt_details: None,
        completion_details: None,
        cache_read_tokens: None,
        numeric_extras: BTreeMap::new(),
    }
}

struct RecordingPricing {
    seen: RefCell<Vec<(String, String, CostUsage)>>,
    fail: bool,
}

impl GenericTokenPricing for RecordingPricing {
    fn generic_cost_per_token(
        &self,
        model: &str,
        custom_llm_provider: &str,
        usage: &CostUsage,
    ) -> Result<GenericCostResult, TokenCostError> {
        self.seen.borrow_mut().push((
            model.to_owned(),
            custom_llm_provider.to_owned(),
            usage.clone(),
        ));
        if self.fail {
            return Err(TokenCostError::UnknownModel);
        }
        Ok(GenericCostResult {
            prompt_cost: Decimal::new(15, 1),
            completion_cost: Decimal::new(25, 1),
        })
    }
}

fn recording(fail: bool) -> RecordingPricing {
    RecordingPricing {
        seen: RefCell::new(Vec::new()),
        fail,
    }
}

#[test]
fn the_generic_function_is_called_with_the_tencent_provider_and_its_two_costs_return() {
    assert_eq!(PROVIDER, "tencent");
    let pricing = recording(false);
    let (prompt, completion) = cost_per_token(&pricing, "hunyuan-turbos", &usage()).unwrap();
    assert_eq!(prompt, Decimal::new(15, 1));
    assert_eq!(completion, Decimal::new(25, 1));
    let seen = pricing.seen.borrow();
    assert_eq!(seen.len(), 1);
    let (model, provider, billed) = seen.first().unwrap();
    assert_eq!(model, "hunyuan-turbos");
    assert_eq!(provider, "tencent");
    assert_eq!(billed.prompt_tokens, 11);
    assert_eq!(billed.completion_tokens, 4);
}

#[test]
fn a_generic_cost_failure_keeps_its_cause() {
    let error = cost_per_token(&recording(true), "hunyuan-turbos", &usage()).unwrap_err();
    assert!(matches!(
        error,
        TencentCostError::Cost {
            source: TokenCostError::UnknownModel
        }
    ));
}

#[test]
fn the_error_names_the_provider_whose_cost_failed() {
    let error = cost_per_token(&recording(true), "hunyuan-turbos", &usage()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "the tencent token cost could not be computed: the model is not registered in the pricing catalog"
    );
}
