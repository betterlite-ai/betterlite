#![cfg(test)]

use betterlite_models::cost_usage::CostUsage;
use betterlite_models::token_cost::GenericCostResult;
use betterlite_models::token_cost::error::TokenCostError;
use rust_decimal::Decimal;

use super::{GenericTokenPricing, PROVIDER, TencentCostError, cost_per_token};

fn usage() -> CostUsage {
    CostUsage {
        prompt_tokens: 11,
        completion_tokens: 4,
        total_tokens: 15,
        ..CostUsage::default()
    }
}

struct FixedPricing;

impl GenericTokenPricing for FixedPricing {
    fn generic_cost_per_token(
        &self,
        model: &str,
        custom_llm_provider: &str,
        billed_usage: &CostUsage,
    ) -> Result<GenericCostResult, TokenCostError> {
        assert_eq!(model, "hunyuan-turbos");
        assert_eq!(custom_llm_provider, PROVIDER);
        assert_eq!(billed_usage.prompt_tokens, 11);
        assert_eq!(billed_usage.completion_tokens, 4);
        Ok(GenericCostResult {
            prompt_cost: Decimal::new(15, 1),
            completion_cost: Decimal::new(25, 1),
        })
    }
}

struct FailingPricing;

impl GenericTokenPricing for FailingPricing {
    fn generic_cost_per_token(
        &self,
        _model: &str,
        _custom_llm_provider: &str,
        _usage: &CostUsage,
    ) -> Result<GenericCostResult, TokenCostError> {
        Err(TokenCostError::UnknownModel)
    }
}

#[test]
fn the_generic_function_is_called_with_the_tencent_provider_and_its_two_costs_return() {
    let (prompt, completion) = cost_per_token(&FixedPricing, "hunyuan-turbos", &usage()).unwrap();
    assert_eq!(prompt, Decimal::new(15, 1));
    assert_eq!(completion, Decimal::new(25, 1));
}

#[test]
fn a_generic_cost_failure_keeps_its_cause() {
    let error = cost_per_token(&FailingPricing, "hunyuan-turbos", &usage()).unwrap_err();
    assert!(matches!(
        error,
        TencentCostError::Cost {
            source: TokenCostError::UnknownModel
        }
    ));
}

#[test]
fn the_error_names_the_provider_whose_cost_failed() {
    let error = cost_per_token(&FailingPricing, "hunyuan-turbos", &usage()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "the tencent token cost could not be computed: the model is not registered in the pricing catalog"
    );
}
