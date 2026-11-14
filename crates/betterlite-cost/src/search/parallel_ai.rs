use std::collections::BTreeMap;
use std::error::Error;

use rust_decimal::Decimal;

use super::SearchPricingSource;
use super::error::SearchCostError;
use super::params::{ProviderUsage, SearchOptionalParams, UsageField};

const DEFAULT_RESULTS: u64 = 10;
const ADDITIONAL_RESULT_COST: Decimal = Decimal::from_parts(1, 0, 0, false, 3);
const STANDARD_SEARCH_MODEL: &str = "parallel_ai/search";
const FAST_SEARCH_MODEL: &str = "parallel_ai/search-fast";
const TURBO_SEARCH_MODEL: &str = "parallel_ai/search-turbo";
const SEARCH_SKU: &str = "sku_search";
const ADDITIONAL_RESULTS_SKU: &str = "sku_search_additional_results";

pub(super) async fn search_cost<Pricing>(
    pricing: &Pricing,
    params: &SearchOptionalParams,
) -> Result<Decimal, SearchCostError<Pricing::Error>>
where
    Pricing: SearchPricingSource,
{
    // processor "pro" reads as mode "advanced", which prices like the standard model, so it needs no branch
    let pricing_model = match params.mode.as_deref() {
        Some("fast") => FAST_SEARCH_MODEL,
        Some("turbo") => TURBO_SEARCH_MODEL,
        Some(_) | None => STANDARD_SEARCH_MODEL,
    };
    let model_info = pricing
        .model_info(pricing_model, Some(super::PARALLEL_AI_PROVIDER))
        .await
        .map_err(|source| SearchCostError::ModelInfo {
            model: pricing_model.to_owned(),
            source,
        })?;
    let request_cost = model_info.input_cost_per_query.unwrap_or(Decimal::ZERO);
    let (request_count, additional_results) = counted_results::<Pricing::Error>(params)?;
    request_count
        .checked_mul(request_cost)
        .zip(additional_results.checked_mul(ADDITIONAL_RESULT_COST))
        .and_then(|(requests, additional)| requests.checked_add(additional))
        .ok_or(SearchCostError::Overflow)
}

fn counted_results<PricingError>(
    params: &SearchOptionalParams,
) -> Result<(Decimal, Decimal), SearchCostError<PricingError>>
where
    PricingError: Error + 'static,
{
    match &params.provider_usage {
        Some(ProviderUsage::Items(items)) => Ok((
            usage_count::<PricingError>(items, SEARCH_SKU)?.unwrap_or(Decimal::ONE),
            usage_count::<PricingError>(items, ADDITIONAL_RESULTS_SKU)?.unwrap_or(Decimal::ZERO),
        )),
        Some(ProviderUsage::Invalid(_)) | None => Ok((
            Decimal::ONE,
            Decimal::from(effective_max_results(params).saturating_sub(DEFAULT_RESULTS)),
        )),
    }
}

// None when no item carries the SKU with a usable count, so the caller keeps its own default
fn usage_count<PricingError>(
    items: &[BTreeMap<String, UsageField>],
    sku: &str,
) -> Result<Option<Decimal>, SearchCostError<PricingError>>
where
    PricingError: Error + 'static,
{
    let mut total = None;
    for item in items {
        let named = matches!(item.get("name"), Some(UsageField::Text(name)) if name == sku);
        let Some(UsageField::Count(count)) = item.get("count") else {
            continue;
        };
        if named {
            let so_far = total.unwrap_or(Decimal::ZERO);
            total = Some(
                so_far
                    .checked_add(Decimal::from(*count))
                    .ok_or(SearchCostError::Overflow)?,
            );
        }
    }
    Ok(total)
}

fn effective_max_results(params: &SearchOptionalParams) -> u64 {
    let advanced = params
        .advanced_settings
        .as_ref()
        .and_then(|settings| settings.max_results);
    [advanced, params.max_results]
        .into_iter()
        .flatten()
        .find_map(|requested| u64::try_from(requested).ok())
        .unwrap_or(DEFAULT_RESULTS)
}
