pub mod cache_default;
mod completion_charge;
pub mod error;
pub mod prompt_overlap;
pub mod regional;
mod tests;

use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;

use cache_default::provider_cache_read_default;
use completion_charge::{completion_cost, completion_tokens_details};
use error::TokenCostError;
use prompt_overlap::{PromptExtras, prompt_tokens_details, repair_prompt_overlap};
use regional::{regional_uplift_multiplier, vertex_endpoint_uplift};

use crate::cost_usage::CostUsage;
use crate::modality_cost::input_cost::{InputCostRates, calculate_input_cost};
use crate::off_peak::schema::OffPeakBlock;
use crate::price_lookup::PriceLookup;
use crate::tiered_pricing::Tier;
use crate::token_rates::base_cost::get_token_base_cost;
use crate::token_rates::{EffectiveRates, ModelInfo, ModelPricing, RequestContext, Usage};

pub struct GenericCostRequest<'request> {
    pub model: &'request str,
    pub custom_llm_provider: Option<&'request str>,
    pub usage: &'request CostUsage,
    pub prompt_extras: Option<&'request PromptExtras>,
    pub supplied_model_info: Option<&'request ModelInfo>,
    pub service_tier: Option<&'request str>,
    pub data_residency: Option<&'request str>,
    pub vertex_location: Option<&'request str>,
    pub moment: DateTime<Utc>,
    pub tiered_pricing: &'request [Tier],
    pub off_peak_pricing: Option<&'request OffPeakBlock>,
    pub off_peak_cache_read_present: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GenericCostResult {
    pub prompt_cost: Decimal,
    pub completion_cost: Decimal,
}

fn resolve_model_info(
    request: &GenericCostRequest<'_>,
    lookup: &dyn PriceLookup,
) -> Result<ModelInfo, TokenCostError> {
    if let Some(supplied) = request.supplied_model_info {
        return Ok(supplied.clone());
    }
    lookup
        .effective_model_info(request.model, request.custom_llm_provider)
        .ok_or(TokenCostError::UnknownModel)
}

struct ResolvedPricing {
    model_info: ModelInfo,
    rates: EffectiveRates,
}

fn resolve_pricing(
    request: &GenericCostRequest<'_>,
    lookup: &dyn PriceLookup,
    database: &TimeZoneDatabase,
) -> Result<ResolvedPricing, TokenCostError> {
    let resolved_model_info = resolve_model_info(request, lookup)?;
    let (model_info, off_peak_pricing) = provider_cache_read_default(
        &resolved_model_info,
        request.custom_llm_provider,
        request.off_peak_pricing,
        request.off_peak_cache_read_present,
    )?;

    let pricing = ModelPricing {
        model: &model_info,
        tiered_pricing: request.tiered_pricing,
        off_peak_pricing: off_peak_pricing.as_ref(),
    };
    let context = RequestContext {
        service_tier: request.service_tier,
        provider: request.custom_llm_provider,
        moment: request.moment,
        database,
    };
    let rates = get_token_base_cost(
        &pricing,
        Usage {
            prompt_tokens: request.usage.prompt_tokens,
        },
        &context,
    )?;

    Ok(ResolvedPricing { model_info, rates })
}

fn prompt_side_cost(
    request: &GenericCostRequest<'_>,
    model_info: &ModelInfo,
    rates: &EffectiveRates,
) -> Result<Decimal, TokenCostError> {
    let parsed_prompt_details = prompt_tokens_details(request.usage, request.prompt_extras)?;
    let prompt_details = repair_prompt_overlap(parsed_prompt_details, request.usage.prompt_tokens)?;
    let input_rates = InputCostRates {
        prompt_base_cost: rates.input_rate,
        cache_read_cost: rates.cache_read_rate,
        cache_creation_cost: rates.cache_creation_rate,
        cache_creation_cost_above_1hr: rates.cache_creation_above_1hr_rate,
    };
    calculate_input_cost(
        &prompt_details,
        model_info,
        &input_rates,
        request.service_tier,
    )
    .map_err(TokenCostError::from)
}

// Ports generic_cost_per_token: resolves one rate set, repairs prompt-side double counting, bills the completion side by its own breakdown, then applies the residency and Vertex uplifts to both totals in that order. regional::provider_geo_multiplier is deliberately not applied here; only the billed-rate call sites that need it invoke it directly.
pub fn generic_cost_per_token(
    request: &GenericCostRequest<'_>,
    lookup: &dyn PriceLookup,
    database: &TimeZoneDatabase,
) -> Result<GenericCostResult, TokenCostError> {
    let ResolvedPricing { model_info, rates } = resolve_pricing(request, lookup, database)?;

    let mut prompt_cost = prompt_side_cost(request, &model_info, &rates)?;
    let (completion_details, is_text_tokens_total) = completion_tokens_details(request.usage)?;
    let mut total_completion_cost = completion_cost(
        &completion_details,
        is_text_tokens_total,
        &model_info,
        &rates,
    )?;

    let residency_multiplier = regional_uplift_multiplier(&model_info, request.data_residency);
    let vertex_multiplier = vertex_endpoint_uplift(&model_info, request.vertex_location);
    let overflow = || TokenCostError::Overflow;
    prompt_cost = prompt_cost
        .checked_mul(residency_multiplier)
        .and_then(|value| value.checked_mul(vertex_multiplier))
        .ok_or_else(overflow)?;
    total_completion_cost = total_completion_cost
        .checked_mul(residency_multiplier)
        .and_then(|value| value.checked_mul(vertex_multiplier))
        .ok_or_else(overflow)?;

    Ok(GenericCostResult {
        prompt_cost,
        completion_cost: total_completion_cost,
    })
}
