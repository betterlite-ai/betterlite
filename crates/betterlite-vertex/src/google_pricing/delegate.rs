use betterlite_models::catalog::CatalogEntry;
use betterlite_models::token_cost::GenericCostResult;
use betterlite_models::token_cost::error::TokenCostError;
use betterlite_models::usage_sum::Usage;
use rust_decimal::Decimal;

pub struct GenericTokenRequest<'request> {
    pub model: &'request str,
    pub custom_llm_provider: &'request str,
    pub usage: &'request Usage,
    pub service_tier: Option<&'request str>,
    pub vertex_location: Option<&'request str>,
}

// B068 supplies both methods; the calculator here never resolves token rates or regional uplift itself.
pub trait GenericTokenPricing {
    fn generic_cost_per_token(
        &self,
        request: &GenericTokenRequest<'_>,
    ) -> Result<GenericCostResult, TokenCostError>;

    fn vertex_endpoint_uplift(
        &self,
        record: &CatalogEntry,
        vertex_location: Option<&str>,
    ) -> Decimal;
}
