use rust_decimal::Decimal;

use crate::cost_usage::server_tool_projection::{ServerToolRecord, web_search_requests};
use crate::decisions::Json;
use crate::price_fields::decimal_lexeme::parse_decimal_lexeme;
use crate::price_fields::nested::SearchContextPricing;
use crate::price_lookup::PriceRecord;

use super::error::AnthropicPricingError;

// cost_calculation.py get_cost_for_anthropic_web_search: model info and a reported count are both required before the medium rate applies.
pub fn web_search_cost(
    model_info: Option<&PriceRecord>,
    search_context: Option<&SearchContextPricing>,
    server_tool_use: Option<ServerToolRecord<'_>>,
) -> Result<Decimal, AnthropicPricingError> {
    if model_info.is_none() {
        return Ok(Decimal::ZERO);
    }
    let Some(count) = web_search_requests(server_tool_use) else {
        return Ok(Decimal::ZERO);
    };
    let rate = search_context
        .and_then(|pricing| pricing.search_context_size_medium)
        .unwrap_or(Decimal::ZERO);
    if rate.is_zero() {
        return Ok(Decimal::ZERO);
    }
    rate.checked_mul(search_count(&count)?)
        .ok_or(AnthropicPricingError::Overflow)
}

// The count reaches the multiplication exactly as the tolerant accessor read it; a value Python could not multiply is a typed failure.
fn search_count(value: &Json) -> Result<Decimal, AnthropicPricingError> {
    match value {
        Json::Number(number) => parse_decimal_lexeme(&number.to_string()).map_err(|_source| {
            AnthropicPricingError::InvalidSearchCount {
                kind: "a number beyond the decimal range",
            }
        }),
        Json::Bool(flag) => Ok(Decimal::from(u8::from(*flag))),
        Json::Null => Err(AnthropicPricingError::InvalidSearchCount { kind: "null" }),
        Json::Text(_) => Err(AnthropicPricingError::InvalidSearchCount { kind: "a string" }),
        Json::List(_) => Err(AnthropicPricingError::InvalidSearchCount { kind: "an array" }),
        Json::Object(_) => Err(AnthropicPricingError::InvalidSearchCount { kind: "an object" }),
    }
}

mod tests;
