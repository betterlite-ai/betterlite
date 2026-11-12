pub mod error;
pub mod probe;
pub mod search;
mod surcharge;

use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;

use crate::cost_usage::CostUsage;
use crate::cost_usage::server_tool_projection::ServerToolProjection;
use crate::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use crate::token_cost::prompt_overlap::PromptExtras;
use crate::token_cost::{GenericCostRequest, GenericCostResult, generic_cost_per_token};

use error::AnthropicPricingError;
use surcharge::apply_surcharges;

pub const ANTHROPIC_PROVIDER: &str = "anthropic";
pub const BEDROCK_PROVIDER: &str = "bedrock";

pub struct AnthropicPriceRequest<'request> {
    pub model: &'request str,
    pub usage: &'request CostUsage,
    pub prompt_extras: &'request PromptExtras,
    pub supplied_model_info: Option<&'request PriceRecord>,
    // The effective model info's nested provider_specific_entry, which PriceRecord cannot carry itself.
    pub provider_specific_entry: Option<&'request PriceRecord>,
    pub service_tier: Option<&'request str>,
    pub projection: &'request ServerToolProjection,
    pub moment: DateTime<Utc>,
}

pub struct BedrockPriceRequest<'request> {
    pub model: &'request str,
    pub usage: &'request CostUsage,
    pub prompt_extras: &'request PromptExtras,
    pub service_tier: Option<&'request str>,
    pub moment: DateTime<Utc>,
}

// anthropic and bedrock cost_per_token differ only in the surcharge block, so the provider is the typed policy and the match stays exhaustive.
pub enum TokenPriceRequest<'request> {
    Anthropic(AnthropicPriceRequest<'request>),
    Bedrock(BedrockPriceRequest<'request>),
}

pub fn token_cost(
    policy: &TokenPriceRequest<'_>,
    lookup: &dyn PriceLookup,
    database: &TimeZoneDatabase,
) -> Result<GenericCostResult, AnthropicPricingError> {
    match policy {
        TokenPriceRequest::Anthropic(request) => anthropic_token_cost(request, lookup, database),
        TokenPriceRequest::Bedrock(request) => {
            let generic = GenericCostRequest {
                model: request.model,
                custom_llm_provider: Some(BEDROCK_PROVIDER),
                usage: request.usage,
                prompt_extras: request.prompt_extras,
                supplied_pricing: None,
                service_tier: request.service_tier,
                data_residency: None,
                vertex_location: None,
                moment: request.moment,
            };
            Ok(generic_cost_per_token(&generic, lookup, database)?)
        }
    }
}

fn anthropic_token_cost(
    request: &AnthropicPriceRequest<'_>,
    lookup: &dyn PriceLookup,
    database: &TimeZoneDatabase,
) -> Result<GenericCostResult, AnthropicPricingError> {
    let supplied_pricing = request.supplied_model_info.map(|record| PricedModel {
        record: record.clone(),
        tiered_pricing: Vec::new(),
        off_peak_pricing: None,
        off_peak_cache_read_present: false,
    });
    let generic = GenericCostRequest {
        model: request.model,
        custom_llm_provider: Some(ANTHROPIC_PROVIDER),
        usage: request.usage,
        prompt_extras: request.prompt_extras,
        supplied_pricing: supplied_pricing.as_ref(),
        service_tier: request.service_tier,
        data_residency: None,
        vertex_location: None,
        moment: request.moment,
    };
    let mut costs = generic_cost_per_token(&generic, lookup, database)?;
    if request.supplied_model_info.is_none()
        && lookup
            .effective_model_info(request.model, Some(ANTHROPIC_PROVIDER))
            .is_none()
    {
        return Ok(costs);
    }
    apply_surcharges(
        request.provider_specific_entry,
        request.projection,
        &mut costs,
    );
    Ok(costs)
}

mod tests;
