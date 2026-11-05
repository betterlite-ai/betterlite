mod error;
mod reconcile;
mod tests;

use std::collections::BTreeMap;

use betterlite_models::cost_usage::CostUsage;
use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use betterlite_models::price_fields::nested::SearchContextPricing;
use betterlite_models::token_cost::GenericCostResult;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde_json::Value;

pub use error::XaiUsageError;
pub use reconcile::{fold_reasoning, normalize_totals, usage_restated_from_xai_ticks};

const USD_TICKS_PER_DOLLAR: u64 = 10_000_000_000;
// https://docs.x.ai/developers/pricing#tools-pricing: five dollars per thousand calls
const DEFAULT_SEARCH_RATE: (i64, u32) = (5, 3);

// xAI usage as the chat transformation sees it: the token counts plus the extras xAI adds to Usage
#[derive(Debug, Clone, Default, PartialEq)]
pub struct XaiUsage {
    pub tokens: CostUsage,
    pub cost: Option<Decimal>,
    pub cost_in_usd_ticks: Option<Value>,
    pub server_side_tool_usage_details: Option<BTreeMap<String, Value>>,
}

pub trait GenericTokenPricing {
    fn generic_cost_per_token(
        &self,
        model: &str,
        custom_llm_provider: &str,
        usage: &CostUsage,
    ) -> Result<GenericCostResult, betterlite_models::token_cost::error::TokenCostError>;
}

// a JSON integer that is not a boolean and not negative, in ticks of a ten-billionth of a dollar
#[must_use]
pub fn xai_reported_cost_in_usd(cost_in_usd_ticks: &Value) -> Option<Decimal> {
    let Value::Number(number) = cost_in_usd_ticks else {
        return None;
    };
    let lexeme = number.to_string();
    let digits = lexeme.strip_prefix('-').unwrap_or(&lexeme);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let ticks = parse_decimal_lexeme(&lexeme).ok()?;
    if ticks < Decimal::ZERO {
        return None;
    }
    ticks.abs().checked_div(Decimal::from(USD_TICKS_PER_DOLLAR))
}

pub fn cost_per_token<P>(
    pricing: &P,
    model: &str,
    usage: &XaiUsage,
) -> Result<(Decimal, Decimal), XaiUsageError>
where
    P: GenericTokenPricing,
{
    if let Some(reported_cost) = usage.cost.filter(|cost| *cost >= Decimal::ZERO) {
        return Ok((Decimal::ZERO, reported_cost));
    }
    let tokens = &usage.tokens;
    let expected_total = tokens
        .prompt_tokens
        .checked_add(tokens.completion_tokens)
        .ok_or(XaiUsageError::Overflow)?;
    let mut adjusted = tokens.clone();
    if tokens.total_tokens != expected_total {
        let reasoning_tokens = tokens
            .completion_details
            .and_then(|details| details.reasoning_tokens)
            .unwrap_or(0);
        adjusted.completion_tokens = tokens
            .completion_tokens
            .checked_add(reasoning_tokens)
            .ok_or(XaiUsageError::Overflow)?;
    }
    adjusted.completion_details = None;
    let cost = pricing.generic_cost_per_token(model, "xai", &adjusted)?;
    Ok((cost.prompt_cost, cost.completion_cost))
}

// a count converts like Python's int(): integers, booleans and numeric text do, decimals truncate, containers do not
#[must_use]
pub fn web_search_calls(details: &BTreeMap<String, Value>) -> Option<i64> {
    match details.get("web_search_calls") {
        None | Some(Value::Null) => Some(0),
        Some(Value::Number(number)) => parse_decimal_lexeme(&number.to_string())
            .ok()?
            .trunc()
            .to_i64(),
        Some(Value::String(text)) if text.is_empty() => Some(0),
        Some(Value::String(text)) => text.trim().parse().ok(),
        Some(Value::Bool(flag)) => Some(i64::from(*flag)),
        Some(Value::Array(_) | Value::Object(_)) => None,
    }
}

pub fn attach_tool_details(usage: &mut XaiUsage, details: Option<BTreeMap<String, Value>>) {
    let Some(details) = details else {
        return;
    };
    let calls = web_search_calls(&details);
    usage.server_side_tool_usage_details = Some(details);
    if let Some(calls) = calls.filter(|calls| *calls > 0) {
        usage
            .tokens
            .prompt_details
            .get_or_insert_default()
            .web_search_requests = Some(calls);
    }
}

pub fn cost_per_web_search_request(
    usage: &XaiUsage,
    search_context: Option<&SearchContextPricing>,
) -> Result<Decimal, XaiUsageError> {
    let Some(details) = &usage.server_side_tool_usage_details else {
        return Ok(Decimal::ZERO);
    };
    if usage.cost.is_some_and(|cost| cost >= Decimal::ZERO) {
        return Ok(Decimal::ZERO);
    }
    let Some(calls) = web_search_calls(details).filter(|calls| *calls > 0) else {
        return Ok(Decimal::ZERO);
    };
    let rate = search_context
        .into_iter()
        .flat_map(|pricing| {
            [
                pricing.search_context_size_medium,
                pricing.search_context_size_low,
                pricing.search_context_size_high,
            ]
        })
        .flatten()
        .find(|rate| *rate > Decimal::ZERO)
        .unwrap_or_else(|| {
            let (mantissa, scale) = DEFAULT_SEARCH_RATE;
            Decimal::new(mantissa, scale)
        });
    rate.checked_mul(Decimal::from(calls))
        .ok_or(XaiUsageError::Overflow)
}
