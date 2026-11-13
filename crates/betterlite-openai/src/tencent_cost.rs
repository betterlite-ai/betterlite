use betterlite_models::cost_usage::CostUsage;
use betterlite_models::token_cost::GenericCostResult;
use betterlite_models::token_cost::error::TokenCostError;
use rust_decimal::Decimal;
use thiserror::Error;

mod tests;

// LiteLLM's tencent cost calculator only pins the provider name; the arithmetic belongs to B1672
pub const PROVIDER: &str = "tencent";

#[derive(Debug, Error)]
pub enum TencentCostError {
    #[error("the tencent token cost could not be computed: {source}")]
    Cost {
        #[source]
        source: TokenCostError,
    },
}

// the caller supplies B1672's generic function, so this crate never depends on its implementation
pub trait GenericTokenPricing {
    fn generic_cost_per_token(
        &self,
        model: &str,
        custom_llm_provider: &str,
        usage: &CostUsage,
    ) -> Result<GenericCostResult, TokenCostError>;
}

pub fn cost_per_token<P>(
    pricing: &P,
    model: &str,
    usage: &CostUsage,
) -> Result<(Decimal, Decimal), TencentCostError>
where
    P: GenericTokenPricing,
{
    let billed = pricing
        .generic_cost_per_token(model, PROVIDER, usage)
        .map_err(|source| TencentCostError::Cost { source })?;
    Ok((billed.prompt_cost, billed.completion_cost))
}
