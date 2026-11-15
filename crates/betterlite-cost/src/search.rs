use std::error::Error;
use std::future::Future;

use rust_decimal::Decimal;

pub mod error;
pub mod model_info;
mod parallel_ai;
pub mod params;
mod tests;

use error::SearchCostError;
use model_info::SearchModelInfo;
use params::SearchOptionalParams;

const DEFAULT_MAX_RESULTS: i64 = 10;
const PARALLEL_AI_PROVIDER: &str = "parallel_ai";

// B2225's get_model_info answered per call, so a mutated cached record shows up in the next cost
pub trait SearchPricingSource: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn model_info(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> impl Future<Output = Result<SearchModelInfo, Self::Error>> + Send;
}

pub async fn search_provider_cost_per_query<Pricing>(
    pricing: &Pricing,
    model: &str,
    custom_llm_provider: Option<&str>,
    number_of_queries: u64,
    optional_params: &SearchOptionalParams,
) -> Result<(Decimal, Decimal), SearchCostError<Pricing::Error>>
where
    Pricing: SearchPricingSource,
{
    if custom_llm_provider == Some(PARALLEL_AI_PROVIDER) {
        let input_cost = parallel_ai::search_cost(pricing, optional_params).await?;
        return Ok((input_cost, Decimal::ZERO));
    }
    let model_info = pricing
        .model_info(model, custom_llm_provider)
        .await
        .map_err(|source| SearchCostError::ModelInfo {
            model: model.to_owned(),
            source,
        })?;
    let tiers = model_info.tiered_pricing.as_deref().unwrap_or_default();
    let cost_per_query = match tiers.last() {
        Some(highest_tier) => {
            let max_results = optional_params.max_results.unwrap_or(DEFAULT_MAX_RESULTS);
            tiers
                .iter()
                .find(|tier| {
                    let (range_min, range_max) = tier.max_results_range;
                    (range_min..=range_max).contains(&max_results)
                })
                .unwrap_or(highest_tier)
                .input_cost_per_query
        }
        None => model_info.input_cost_per_query.unwrap_or(Decimal::ZERO),
    };
    let input_cost = Decimal::from(number_of_queries)
        .checked_mul(cost_per_query)
        .ok_or(SearchCostError::Overflow)?;
    Ok((input_cost, Decimal::ZERO))
}
