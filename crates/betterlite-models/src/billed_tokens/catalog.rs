use crate::modality_cost::pricing::price_for_key;
use crate::modality_cost::pricing::service_tier_cost_key;
use crate::price_lookup::PriceRecord;
use crate::token_rates::EffectiveRates;
use jiff::tz::TimeZoneDatabase;

use super::{BilledTokenError, BilledTokenRates, BilledTokenRequest};
use crate::price_lookup::{PriceLookup, PricedModel};
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
    let Ok(defaulted) = default_pricing(&priced, request.provider) else {
        return Ok(None);
    };
    map_billed_rates(request, &defaulted, database).map(Some)
}

pub fn default_pricing(
    priced: &PricedModel,
    provider: Option<&str>,
) -> Result<PricedModel, BilledTokenError> {
    let (record, off_peak_pricing) = provider_cache_read_default(
        &priced.record,
        provider,
        priced.off_peak_pricing.as_ref(),
        priced.off_peak_cache_read_present,
    )?;
    Ok(PricedModel {
        record,
        off_peak_pricing,
        tiered_pricing: priced.tiered_pricing.clone(),
        off_peak_cache_read_present: priced.off_peak_cache_read_present,
    })
}

pub fn base_rates(
    request: &BilledTokenRequest<'_>,
    priced: &PricedModel,
    database: &TimeZoneDatabase,
) -> Result<EffectiveRates, BilledTokenError> {
    let pricing = ModelPricing {
        model: &priced.record,
        tiered_pricing: &priced.tiered_pricing,
        off_peak_pricing: priced.off_peak_pricing.as_ref(),
    };
    let context = RequestContext {
        service_tier: request.service_tier,
        provider: request.provider,
        moment: request.moment,
        database,
    };
    Ok(get_token_base_cost(
        &pricing,
        Usage {
            prompt_tokens: request.prompt_tokens,
        },
        &context,
    )?)
}

pub fn map_billed_rates(
    request: &BilledTokenRequest<'_>,
    priced: &PricedModel,
    database: &TimeZoneDatabase,
) -> Result<BilledTokenRates, BilledTokenError> {
    let model = &priced.record;
    let rates = base_rates(request, priced, database)?;
    let audio_key =
        service_tier_cost_key("cache_read_input_audio_token_cost", request.service_tier);
    let audio = price_for_key(model, &audio_key)?.unwrap_or(rates.cache_read_rate);
    let multiplier = billing_multiplier(request, model)?;
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
    Ok(billed_rates)
}

fn billing_multiplier(
    request: &BilledTokenRequest<'_>,
    model: &PriceRecord,
) -> Result<rust_decimal::Decimal, BilledTokenError> {
    let geo = provider_geo_multiplier(request.provider_specific_entry, request.inference_geo)?;
    regional_uplift_multiplier(model, request.residency)
        .checked_mul(vertex_endpoint_uplift(model, request.vertex_location))
        .and_then(|value| value.checked_mul(geo))
        .ok_or(BilledTokenError::Overflow)
}
