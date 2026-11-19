use std::collections::BTreeMap;

use betterlite_anthropic::cache_aware_routing::services::PriceRequest;
use betterlite_anthropic::cache_aware_routing::types::CacheTokenBuckets;
use betterlite_models::anthropic_pricing::ANTHROPIC_PROVIDER;
use betterlite_models::cost_usage::CostUsage;
use betterlite_models::cost_usage::PromptDetails;
use betterlite_models::cost_usage::modality_projection::CacheCreationTokenDetails;
use betterlite_models::price_lookup::PriceRecord;
use betterlite_models::price_lookup::PricedModel;
use betterlite_models::token_cost::GenericCostRequest;
use betterlite_models::token_cost::generic_cost_per_token;
use betterlite_models::token_cost::prompt_overlap::PromptExtras;
use chrono::DateTime;
use chrono::Utc;
use rust_decimal::Decimal;

use crate::cost::prompt_cache_scenarios::PriceEnvironment;
use crate::cost::prompt_cache_scenarios::pricing_validation::selected_prices;
use crate::cost::prompt_cache_scenarios::required_prices::has_required_prices;

// Ports price_cache_tokens: the deployment tariff first, every required rate checked, then completion_cost's input and output sums.
#[must_use]
pub fn price_cache_tokens(
    environment: &PriceEnvironment<'_>,
    moment: DateTime<Utc>,
    request: &PriceRequest<'_>,
) -> Option<Decimal> {
    let (selected, prices) = selected_prices(environment, request.model, request.deployment_id)?;
    if request.completion_tokens < 0
        || !has_required_prices(&prices, request.tokens, request.completion_tokens)
    {
        return None;
    }
    priced_cost(environment, moment, request, &selected, prices)
}

fn priced_cost(
    environment: &PriceEnvironment<'_>,
    moment: DateTime<Utc>,
    request: &PriceRequest<'_>,
    selected: &str,
    prices: PriceRecord,
) -> Option<Decimal> {
    let usage = cost_usage(request);
    let extras = prompt_extras(request.tokens);
    let pricing = PricedModel {
        record: prices,
        tiered_pricing: Vec::new(),
        off_peak_pricing: None,
        off_peak_cache_read_present: false,
    };
    let generic = GenericCostRequest {
        model: selected,
        custom_llm_provider: Some(ANTHROPIC_PROVIDER),
        usage: &usage,
        prompt_extras: &extras,
        supplied_pricing: Some(&pricing),
        service_tier: None,
        data_residency: None,
        vertex_location: None,
        moment,
    };
    let costs = generic_cost_per_token(&generic, environment.lookup, environment.database).ok()?;
    let cost = costs.prompt_cost.checked_add(costs.completion_cost)?;
    if cost.is_sign_negative() {
        return None;
    }
    Some(cost)
}

fn cost_usage(request: &PriceRequest<'_>) -> CostUsage {
    let tokens = request.tokens;
    let prompt_tokens = tokens.total_tokens();
    CostUsage {
        prompt_tokens,
        completion_tokens: request.completion_tokens,
        total_tokens: prompt_tokens.saturating_add(request.completion_tokens),
        prompt_details: Some(PromptDetails::default()),
        completion_details: None,
        cache_read_tokens: Some(tokens.cache_read_input_tokens),
        numeric_extras: BTreeMap::new(),
    }
}

fn prompt_extras(tokens: &CacheTokenBuckets) -> PromptExtras {
    let five_minute = tokens.cache_creation_5m_input_tokens;
    let one_hour = tokens.cache_creation_1h_input_tokens;
    PromptExtras {
        cache_write_tokens: None,
        cache_creation_tokens: Some(five_minute.saturating_add(one_hour)),
        cache_creation_token_details: Some(CacheCreationTokenDetails {
            ephemeral_5m_input_tokens: Some(five_minute),
            ephemeral_1h_input_tokens: Some(one_hour),
        }),
        cached_tokens_details: None,
        character_count: None,
        image_count: None,
        video_length_seconds: None,
        audio_length_seconds: None,
        query_count: None,
    }
}

mod tests;
