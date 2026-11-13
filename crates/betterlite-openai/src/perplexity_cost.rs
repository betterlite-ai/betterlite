mod tests;

use betterlite_models::cost_usage::CostUsage;
use betterlite_models::off_peak::rate::OffPeakRateError;
use betterlite_models::off_peak::schema::{OffPeakBlock, OffPeakBlockEntry};
use betterlite_models::off_peak::{TokenRates, apply_off_peak_pricing};
use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use betterlite_models::token_cost::GenericCostResult;
use betterlite_prompts::arguments::json::{Json, Object};
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;
use thiserror::Error;

use crate::perplexity_chat::json_truthy;

#[derive(Debug, Error)]
pub enum PerplexityCostError {
    #[error("the reported Perplexity cost is not a number")]
    ReportedCost,
    #[error("Perplexity cost arithmetic overflowed")]
    Overflow,
    #[error("the off-peak pricing block could not be read")]
    OffPeakPricing(#[source] serde_json::Error),
    #[error(transparent)]
    OffPeak(#[from] OffPeakRateError),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PerplexityUsage {
    pub tokens: CostUsage,
    pub cost: Option<Json>,
    pub citation_tokens: Option<i64>,
    pub reasoning_tokens: Option<i64>,
}

// The caller resolves the model info for provider perplexity; this never looks the catalog up
pub fn cost_per_token(
    model_info: &Object,
    usage: &PerplexityUsage,
    current_time: DateTime<Utc>,
    database: &TimeZoneDatabase,
) -> Result<GenericCostResult, PerplexityCostError> {
    if let Some(total) = reported_total(usage.cost.as_ref())? {
        return Ok(GenericCostResult {
            prompt_cost: Decimal::ZERO,
            completion_cost: total,
        });
    }
    let rates = off_peak_rates(model_info, current_time, database)?;
    let mut prompt_cost = charge(usage.tokens.prompt_tokens, rates.input_rate)?;
    let citation_tokens = usage.citation_tokens.unwrap_or(0);
    if citation_tokens > 0 {
        let citation_rate = rate(model_info, "citation_cost_per_token");
        prompt_cost = prompt_cost
            .checked_add(charge(citation_tokens, citation_rate)?)
            .ok_or(PerplexityCostError::Overflow)?;
    }
    let mut completion_cost = completion_charge(model_info, usage, rates.output_rate)?;
    let search_requests = usage
        .tokens
        .prompt_details
        .and_then(|details| details.web_search_requests)
        .unwrap_or(0);
    if search_requests > 0
        && let Some(search_rate) = search_rate(model_info)
    {
        completion_cost = completion_cost
            .checked_add(charge(search_requests, search_rate)?)
            .ok_or(PerplexityCostError::Overflow)?;
    }
    Ok(GenericCostResult {
        prompt_cost,
        completion_cost,
    })
}

fn reported_total(cost: Option<&Json>) -> Result<Option<Decimal>, PerplexityCostError> {
    match cost {
        Some(Json::Object(fields)) => match fields.get("total_cost") {
            None | Some(Json::Null) => Ok(None),
            Some(value) => parse_rate(value)
                .map(Some)
                .ok_or(PerplexityCostError::ReportedCost),
        },
        Some(Json::Number(number)) => parse_decimal_lexeme(&number.to_string())
            .map(Some)
            .map_err(|_error| PerplexityCostError::ReportedCost),
        None | Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_)) => Ok(None),
    }
}

fn off_peak_rates(
    model_info: &Object,
    current_time: DateTime<Utc>,
    database: &TimeZoneDatabase,
) -> Result<TokenRates, PerplexityCostError> {
    let base = TokenRates {
        input_rate: rate(model_info, "input_cost_per_token"),
        output_rate: rate(model_info, "output_cost_per_token"),
        cache_read_rate: Decimal::ZERO,
        cache_creation_rate: Decimal::ZERO,
        reasoning_rate: None,
    };
    let block = off_peak_block(model_info)?;
    Ok(apply_off_peak_pricing(
        block.as_ref(),
        current_time,
        base,
        database,
    )?)
}

fn off_peak_block(model_info: &Object) -> Result<Option<OffPeakBlock>, PerplexityCostError> {
    let Some(value) = model_info.get("off_peak_pricing") else {
        return Ok(None);
    };
    if matches!(value, Json::Null) {
        return Ok(None);
    }
    let json = serde_json::to_value(value).map_err(PerplexityCostError::OffPeakPricing)?;
    let entry: OffPeakBlockEntry =
        serde_json::from_value(json).map_err(PerplexityCostError::OffPeakPricing)?;
    Ok(entry.as_block().cloned())
}

fn completion_charge(
    model_info: &Object,
    usage: &PerplexityUsage,
    output_rate: Decimal,
) -> Result<Decimal, PerplexityCostError> {
    let mut reasoning_tokens = usage.reasoning_tokens.unwrap_or(0);
    if reasoning_tokens == 0 {
        reasoning_tokens = usage
            .tokens
            .completion_details
            .and_then(|details| details.reasoning_tokens)
            .unwrap_or(0);
    }
    let configured = !matches!(
        model_info.get("output_cost_per_reasoning_token"),
        None | Some(Json::Null)
    );
    if reasoning_tokens > 0 && configured {
        let non_reasoning = usage
            .tokens
            .completion_tokens
            .saturating_sub(reasoning_tokens)
            .max(0);
        let reasoning_rate = rate(model_info, "output_cost_per_reasoning_token");
        let plain = charge(non_reasoning, output_rate)?;
        return plain
            .checked_add(charge(reasoning_tokens, reasoning_rate)?)
            .ok_or(PerplexityCostError::Overflow);
    }
    charge(usage.tokens.completion_tokens, output_rate)
}

fn search_rate(model_info: &Object) -> Option<Decimal> {
    let chosen = model_info
        .get("search_queries_cost_per_query")
        .filter(|value| json_truthy(value))
        .or_else(|| model_info.get("search_context_cost_per_query"))
        .filter(|value| !matches!(value, Json::Null))?;
    Some(match chosen {
        Json::Object(fields) => fields
            .get("search_context_size_low")
            .and_then(parse_rate)
            .unwrap_or(Decimal::ZERO),
        other => parse_rate(other).unwrap_or(Decimal::ZERO),
    })
}

fn rate(model_info: &Object, key: &str) -> Decimal {
    model_info
        .get(key)
        .and_then(parse_rate)
        .unwrap_or(Decimal::ZERO)
}

fn parse_rate(value: &Json) -> Option<Decimal> {
    match value {
        Json::Number(number) => parse_decimal_lexeme(&number.to_string()).ok(),
        Json::Text(text) => parse_decimal_lexeme(text.trim()).ok(),
        Json::Null | Json::Bool(_) | Json::List(_) | Json::Object(_) => None,
    }
}

fn charge(tokens: i64, unit_rate: Decimal) -> Result<Decimal, PerplexityCostError> {
    Decimal::from(tokens)
        .checked_mul(unit_rate)
        .ok_or(PerplexityCostError::Overflow)
}
