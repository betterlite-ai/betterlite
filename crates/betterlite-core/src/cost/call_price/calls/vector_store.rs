use betterlite_models::resolution::{ResolutionRequest, resolve};
use betterlite_models::token_cost::GenericCostResult;
use betterlite_openai::api::vector_store_records::records::vector::VectorStoreSearchResponse;
use rust_decimal::Decimal;

use crate::cost::call_price::CallPriceEnv;
use crate::cost::call_price::error::CallPriceError;
use crate::cost::call_price::policy::VectorStorePriceRequest;

// Ports vector_store_search_cost: a slash in the model re-resolves the provider, and no vector configuration prices at zero.
pub fn vector_store_search_cost(
    model: Option<&str>,
    custom_llm_provider: Option<&str>,
    response: &VectorStoreSearchResponse,
    env: &CallPriceEnv<'_>,
) -> Result<GenericCostResult, CallPriceError> {
    let resolved = model
        .filter(|name| name.contains('/'))
        .map(|slashed| {
            resolve(
                &ResolutionRequest {
                    model: slashed,
                    custom_llm_provider: None,
                    api_base: None,
                    api_key: None,
                },
                env.resolution,
            )
        })
        .transpose()
        .map_err(|source| CallPriceError::Resolution { source })?;
    let provider = resolved.as_ref().map_or_else(
        || custom_llm_provider.unwrap_or("openai"),
        |found| found.provider.slug(),
    );
    let priced = env.policy.vector_store_cost(&VectorStorePriceRequest {
        provider,
        api_type: resolved.as_ref().map(|found| found.model.as_str()),
        response,
    })?;
    Ok(priced.unwrap_or(GenericCostResult {
        prompt_cost: Decimal::ZERO,
        completion_cost: Decimal::ZERO,
    }))
}
