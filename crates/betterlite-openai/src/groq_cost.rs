use betterlite_models::price_fields::nested::SearchContextPricing;
use rust_decimal::Decimal;
use thiserror::Error;

mod tests;

// GROQ_BROWSER_VISIT_WEBSITE_COST_PER_CALL is 1.0 / 1000 in litellm/constants.py L471
const BROWSER_VISIT_SCALE: u32 = 3;

#[derive(Debug, Error)]
pub enum GroqCostError {
    #[error("the Groq web search cost overflows a Decimal")]
    Overflow,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ServerToolUse {
    pub web_search_requests: Option<u64>,
    pub browser_open_requests: Option<u64>,
}

pub fn web_search_cost(
    server_tool_use: Option<&ServerToolUse>,
    search_context_cost_per_query: Option<&SearchContextPricing>,
) -> Result<Decimal, GroqCostError> {
    let Some(usage) = server_tool_use else {
        return Ok(Decimal::ZERO);
    };
    let rate = search_context_cost_per_query
        .and_then(|pricing| pricing.search_context_size_medium)
        .unwrap_or(Decimal::ZERO);
    let searches = Decimal::from(usage.web_search_requests.unwrap_or(0));
    let opens = Decimal::from(usage.browser_open_requests.unwrap_or(0));
    let search_cost = rate.checked_mul(searches).ok_or(GroqCostError::Overflow)?;
    let open_cost = Decimal::new(1, BROWSER_VISIT_SCALE)
        .checked_mul(opens)
        .ok_or(GroqCostError::Overflow)?;
    search_cost
        .checked_add(open_cost)
        .ok_or(GroqCostError::Overflow)
}
