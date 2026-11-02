use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;

use super::error::DurationCostError;
use crate::cost_usage::CostUsage;
use crate::price_lookup::PriceLookup;
use crate::token_cost::prompt_overlap::PromptExtras;
use crate::token_cost::{GenericCostRequest, GenericCostResult, generic_cost_per_token};

#[derive(Clone, Copy)]
pub enum Provider<'request> {
    OpenAi {
        data_residency: Option<&'request str>,
    },
    // response_time_ms matches the source azure signature's own unused parameter: never read there either.
    Azure {
        response_time_ms: Option<Decimal>,
    },
}

pub struct TokenForwardingRequest<'request> {
    pub model: &'request str,
    pub usage: &'request CostUsage,
    pub service_tier: Option<&'request str>,
    pub moment: DateTime<Utc>,
}

pub fn cost_per_token(
    request: &TokenForwardingRequest<'_>,
    provider: Provider<'_>,
    lookup: &dyn PriceLookup,
    database: &TimeZoneDatabase,
) -> Result<GenericCostResult, DurationCostError> {
    let prompt_extras = PromptExtras::default();
    let (custom_llm_provider, data_residency) = match provider {
        Provider::OpenAi { data_residency } => ("openai", data_residency),
        Provider::Azure { .. } => ("azure", None),
    };
    let generic_request = GenericCostRequest {
        model: request.model,
        custom_llm_provider: Some(custom_llm_provider),
        usage: request.usage,
        prompt_extras: &prompt_extras,
        supplied_pricing: None,
        service_tier: request.service_tier,
        data_residency,
        vertex_location: None,
        moment: request.moment,
    };
    Ok(generic_cost_per_token(&generic_request, lookup, database)?)
}

mod tests;
