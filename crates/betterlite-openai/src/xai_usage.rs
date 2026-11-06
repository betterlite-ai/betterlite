pub mod error;
pub mod reconcile;
mod tests;

use std::collections::BTreeMap;

use betterlite_models::cost_usage::CostUsage;
use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use betterlite_models::price_fields::nested::SearchContextPricing;
use betterlite_models::token_cost::GenericCostResult;
use betterlite_models::token_cost::error::TokenCostError;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde_json::Value;

use error::XaiUsageError;

const USD_TICKS_PER_DOLLAR: u64 = 10_000_000_000;

// xAI usage as the chat transformation sees it: the token counts plus the extras xAI adds
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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
    ) -> Result<GenericCostResult, TokenCostError>;
}

// a JSON integer that is not a boolean and not negative, in ticks of a ten-billionth of a dollar
#[must_use]
pub fn xai_reported_cost_in_usd(ticks: &Value) -> Option<Decimal> {
    let Value::Number(number) = ticks else {
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
    ticks.checked_div(Decimal::from(USD_TICKS_PER_DOLLAR))
}

pub fn cost_per_token<P>(
    pricing: &P,
    model: &str,
    usage: &XaiUsage,
) -> Result<(Decimal, Decimal), XaiUsageError>
where
    P: GenericTokenPricing,
{
    if let Some(reported) = usage.cost.filter(|cost| *cost >= Decimal::ZERO) {
        return Ok((Decimal::ZERO, reported));
    }
    let tokens = &usage.tokens;
    let mut adjusted = tokens.clone();
    let expected = tokens
        .prompt_tokens
        .checked_add(tokens.completion_tokens)
        .ok_or(XaiUsageError::Overflow)?;
    if tokens.total_tokens != expected {
        let reasoning = tokens
            .completion_details
            .and_then(|details| details.reasoning_tokens)
            .unwrap_or(0);
        adjusted.completion_tokens = tokens
            .completion_tokens
            .checked_add(reasoning)
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
    let Some(attached) = details else {
        return;
    };
    if let Some(count) = web_search_calls(&attached).filter(|count| *count > 0) {
        usage
            .tokens
            .prompt_details
            .get_or_insert_default()
            .web_search_requests = Some(count);
    }
    usage.server_side_tool_usage_details = Some(attached);
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
    // xAI lists web search at five dollars per thousand calls; medium, low, high is LiteLLM's order
    let fallback = Decimal::new(5, 3);
    let configured = search_context.map_or([None; 3], |pricing| {
        [
            pricing.search_context_size_medium,
            pricing.search_context_size_low,
            pricing.search_context_size_high,
        ]
    });
    let rate = configured
        .into_iter()
        .flatten()
        .find(|rate| *rate > Decimal::ZERO)
        .unwrap_or(fallback);
    rate.checked_mul(Decimal::from(calls))
        .ok_or(XaiUsageError::Overflow)
}
