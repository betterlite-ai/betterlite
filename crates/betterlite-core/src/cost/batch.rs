pub mod error;
mod legacy;
pub mod prompt;
mod tests;

use rust_decimal::Decimal;

use betterlite_models::cost_usage::CostUsage;
use betterlite_models::cost_usage::modality_projection::PromptModalityUsage;
use betterlite_models::price_lookup::PriceLookup;
use betterlite_models::resolution::{ResolutionContext, ResolutionRequest, resolve};
use betterlite_models::tiered_pricing::Scalar;
use betterlite_models::token_cost::prompt_overlap::PromptExtras;
use betterlite_models::token_cost::regional::regional_uplift_multiplier;
use betterlite_models::token_rates::batch::get_batch_cost_rates;
use betterlite_models::token_rates::component::calculate_cost_component;
use betterlite_models::token_rates::{BatchCostRates, ModelInfo, TokenRateError, Usage};

use error::BatchCostError;
use legacy::legacy_prompt_cost;
use prompt::{PromptCacheRates, batch_prompt_cost};

// Only these four fields prove a supplied record holds prices; modality or cache prices alone do not.
const PRIMARY_RATE_KEYS: [&str; 4] = [
    "input_cost_per_token_batches",
    "input_cost_per_token",
    "output_cost_per_token_batches",
    "output_cost_per_token",
];
const INPUT_RATE_KEY: &str = "input_cost_per_token";
const OUTPUT_RATE_KEY: &str = "output_cost_per_token";

pub struct BatchCostRequest<'request> {
    pub model: &'request str,
    pub custom_llm_provider: Option<&'request str>,
    pub usage: &'request CostUsage,
    pub prompt_extras: &'request PromptExtras,
    pub supplied_model_info: Option<&'request ModelInfo>,
    pub data_residency: Option<&'request str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatchCostResult {
    pub prompt_cost: Decimal,
    pub completion_cost: Decimal,
}

pub fn batch_cost_per_token(
    request: &BatchCostRequest<'_>,
    lookup: &dyn PriceLookup,
    resolution: &ResolutionContext<'_>,
) -> Result<BatchCostResult, BatchCostError> {
    let resolved = resolve(
        &ResolutionRequest {
            model: request.model,
            custom_llm_provider: request.custom_llm_provider,
            api_base: None,
            api_key: None,
        },
        resolution,
    )
    .map_err(|source| BatchCostError::Resolution { source })?;
    let provider = resolved.provider.slug();
    let model_info = select_model_info(request, lookup, provider);
    if model_info.is_empty() {
        return Ok(BatchCostResult {
            prompt_cost: Decimal::ZERO,
            completion_cost: Decimal::ZERO,
        });
    }
    let rates = get_batch_cost_rates(
        &model_info,
        Usage {
            prompt_tokens: request.usage.prompt_tokens,
        },
        Some(provider),
    )?;
    let prompt_cost = prompt_side_cost(request, &model_info, rates)?;
    let completion_cost = match rates.output {
        Some(output_rate) => Decimal::from(request.usage.completion_tokens)
            .checked_mul(output_rate)
            .ok_or(BatchCostError::Overflow)?,
        None => ordinary_completion_cost(request.usage, &model_info)?,
    };
    apply_uplift(
        request.data_residency,
        &model_info,
        BatchCostResult {
            prompt_cost,
            completion_cost,
        },
    )
}

fn prompt_side_cost(
    request: &BatchCostRequest<'_>,
    model_info: &ModelInfo,
    rates: BatchCostRates,
) -> Result<Decimal, BatchCostError> {
    let Some(input_rate) = rates.input else {
        let Some(ordinary_rate) = present_rate(model_info, INPUT_RATE_KEY)? else {
            return Ok(Decimal::ZERO);
        };
        return legacy_prompt_cost(
            request.usage,
            request.prompt_extras,
            model_info,
            ordinary_rate,
        );
    };
    let cache_rates = PromptCacheRates {
        read: rates.cache_read,
        creation: rates.cache_creation,
    };
    batch_prompt_cost(
        request.usage,
        request.prompt_extras,
        model_info,
        input_rate,
        cache_rates,
    )
}

fn ordinary_completion_cost(
    usage: &CostUsage,
    model_info: &ModelInfo,
) -> Result<Decimal, BatchCostError> {
    let Some(output_rate) = present_rate(model_info, OUTPUT_RATE_KEY)? else {
        return Ok(Decimal::ZERO);
    };
    Decimal::from(usage.completion_tokens)
        .checked_mul(output_rate)
        .and_then(|cost| cost.checked_div(Decimal::TWO))
        .ok_or(BatchCostError::Overflow)
}

// A zero or absent ordinary rate is no price, so only a truthy one selects a legacy half-rate charge.
fn present_rate(model_info: &ModelInfo, key: &str) -> Result<Option<Decimal>, TokenRateError> {
    let rate = calculate_cost_component(model_info, key, Some(Decimal::ONE))?;
    if rate.is_zero() {
        return Ok(None);
    }
    Ok(Some(rate))
}

fn apply_uplift(
    data_residency: Option<&str>,
    model_info: &ModelInfo,
    result: BatchCostResult,
) -> Result<BatchCostResult, BatchCostError> {
    let uplift = regional_uplift_multiplier(model_info, data_residency);
    if uplift == Decimal::ONE {
        return Ok(result);
    }
    Ok(BatchCostResult {
        prompt_cost: result
            .prompt_cost
            .checked_mul(uplift)
            .ok_or(BatchCostError::Overflow)?,
        completion_cost: result
            .completion_cost
            .checked_mul(uplift)
            .ok_or(BatchCostError::Overflow)?,
    })
}

// A supplied record without prices is only replaced by a nonempty public record; a failed lookup keeps it as it is.
#[must_use]
fn select_model_info(
    request: &BatchCostRequest<'_>,
    lookup: &dyn PriceLookup,
    provider: &str,
) -> ModelInfo {
    match request.supplied_model_info {
        Some(supplied) if has_primary_rate(supplied) => supplied.clone(),
        Some(supplied) => lookup
            .effective_model_info(request.model, Some(provider))
            .filter(|found| !found.is_empty())
            .unwrap_or_else(|| supplied.clone()),
        None => lookup
            .effective_model_info(request.model, Some(provider))
            .unwrap_or_default(),
    }
}

#[must_use]
fn has_primary_rate(model_info: &ModelInfo) -> bool {
    PRIMARY_RATE_KEYS
        .iter()
        .any(|key| !matches!(model_info.get(*key), None | Some(Scalar::Null)))
}

#[must_use]
pub(super) fn prompt_modality_usage(
    usage: &CostUsage,
    extras: &PromptExtras,
) -> PromptModalityUsage {
    PromptModalityUsage {
        base: usage.prompt_details.unwrap_or_default(),
        cached_tokens: usage.cache_read_tokens,
        cache_write_tokens: extras.cache_write_tokens,
        cache_creation_tokens: extras.cache_creation_tokens,
        cache_creation_token_details: extras.cache_creation_token_details,
        cached_tokens_details: extras.cached_tokens_details,
        character_count: extras.character_count,
        image_count: extras.image_count,
        video_length_seconds: extras.video_length_seconds,
        audio_length_seconds: extras.audio_length_seconds,
        query_count: extras.query_count,
    }
}
