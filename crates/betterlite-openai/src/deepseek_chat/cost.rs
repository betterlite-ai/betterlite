use betterlite_models::cost_usage::CostUsage;
use betterlite_models::token_cost::GenericCostResult;
use betterlite_models::token_cost::error::TokenCostError;

use super::PROVIDER;

// B068 supplies the generic calculator; DeepSeek's calculator is that function with this provider tag
pub trait GenericPricing {
    fn generic_cost_per_token(
        &self,
        model: &str,
        usage: &CostUsage,
        provider: Option<&str>,
    ) -> Result<GenericCostResult, TokenCostError>;

    fn cost_per_token(
        &self,
        model: &str,
        usage: &CostUsage,
    ) -> Result<GenericCostResult, TokenCostError> {
        self.generic_cost_per_token(model, usage, Some(PROVIDER))
    }
}
