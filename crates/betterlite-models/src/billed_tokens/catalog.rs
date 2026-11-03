use jiff::tz::TimeZoneDatabase;

use super::{BilledTokenError, BilledTokenRates, BilledTokenRequest};
use crate::price_lookup::PriceLookup;
use crate::token_cost::cache_default::provider_cache_read_default;
use crate::token_cost::regional::{
    provider_geo_multiplier, regional_uplift_multiplier, vertex_endpoint_uplift,
};
use crate::token_rates::base_cost::get_token_base_cost;
use crate::token_rates::{ModelPricing, RequestContext, Usage};

pub fn catalog_billed_token_rates(
    request: &BilledTokenRequest<'_>,
    lookup: &dyn PriceLookup,
    database: &TimeZoneDatabase,
) -> Result<Option<BilledTokenRates>, BilledTokenError> {
    let priced = match request.pricing {
        Some(pricing) => pricing.clone(),
        None => match lookup.effective_priced_model(request.model, request.provider) {
            Some(pricing) => pricing,
            None => return Ok(None),
        },
    };
    let (model, off_peak) = match provider_cache_read_default(
        &priced.record,
        request.provider,
        priced.off_peak_pricing.as_ref(),
        priced.off_peak_cache_read_present,
    ) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    let pricing = ModelPricing {
        model: &model,
        tiered_pricing: &priced.tiered_pricing,
        off_peak_pricing: off_peak.as_ref(),
    };
    let context = RequestContext {
        service_tier: request.service_tier,
        provider: request.provider,
        moment: request.moment,
        database,
    };
    let rates = get_token_base_cost(
        &pricing,
        Usage {
            prompt_tokens: request.prompt_tokens,
        },
        &context,
    )?;
    let audio =
        crate::modality_cost::pricing::price_for_key(&model, "cache_read_input_audio_token_cost")?
            .unwrap_or(rates.cache_read_rate);
    let geo = provider_geo_multiplier(request.provider_specific_entry, request.inference_geo)?;
    let multiplier = regional_uplift_multiplier(&model, request.residency)
        .checked_mul(vertex_endpoint_uplift(&model, request.vertex_location))
        .and_then(|value| value.checked_mul(geo))
        .ok_or(BilledTokenError::Overflow)?;
    let billed_rates = BilledTokenRates {
        input_cost_per_token: rates.input_rate,
        output_cost_per_token: rates.output_rate,
        cache_read_input_token_cost: rates.cache_read_rate,
        audio_read: audio,
        cache_write: rates.cache_creation_rate,
        cache_write_1h: rates.cache_creation_above_1hr_rate,
        reasoning: rates.reasoning_rate,
    }
    .scaled(multiplier)?;
    Ok(Some(billed_rates))
}
