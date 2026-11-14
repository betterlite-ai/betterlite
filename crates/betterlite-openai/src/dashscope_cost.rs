use betterlite_models::cost_usage::CostUsage;
use betterlite_models::modality_cost::completion::parse_completion_details;
use betterlite_models::modality_cost::error::ModalityCostError;
use betterlite_models::off_peak::rate::OffPeakRateError;
use betterlite_models::off_peak::{TokenRates, apply_off_peak_pricing};
use betterlite_models::price_lookup::PricedModel;
use betterlite_models::tiered_pricing::{
    Scalar, Tier, TieredPricingError, coerce_cost_per_token, select_tier_for_input, tier_rate,
};
use betterlite_models::token_cost::GenericCostResult;
use betterlite_models::token_cost::error::TokenCostError;
use betterlite_models::token_cost::prompt_overlap::{PromptExtras, prompt_tokens_details};
use betterlite_models::token_rates::ModelInfo;
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;

const INPUT_COST: &str = "input_cost_per_token";
const OUTPUT_COST: &str = "output_cost_per_token";
const CACHE_READ_COST: &str = "cache_read_input_token_cost";
const CACHE_CREATION_COST: &str = "cache_creation_input_token_cost";
const REASONING_COST: &str = "output_cost_per_reasoning_token";

#[derive(Debug, thiserror::Error)]
pub enum DashscopeCostError {
    #[error(transparent)]
    Prompt(#[from] TokenCostError),
    #[error(transparent)]
    Details(#[from] ModalityCostError),
    #[error(transparent)]
    Tier(#[from] TieredPricingError),
    #[error(transparent)]
    OffPeak(#[from] OffPeakRateError),
    #[error("the DashScope token breakdown overflowed the billing arithmetic")]
    Overflow,
}

pub struct CostRequest<'request> {
    pub model_info: &'request PricedModel,
    pub usage: &'request CostUsage,
    pub prompt_extras: &'request PromptExtras,
    pub current_time: DateTime<Utc>,
}

// one request is billed at a single tier, so every class of token is counted before the rates are picked
struct Breakdown {
    text: i64,
    cached: i64,
    cache_creation: i64,
    completion: i64,
    reasoning: i64,
}

impl Breakdown {
    fn total_input(&self) -> Result<i64, DashscopeCostError> {
        self.text
            .checked_add(self.cached)
            .and_then(|total| total.checked_add(self.cache_creation))
            .ok_or(DashscopeCostError::Overflow)
    }
}

pub fn cost_per_token(
    request: &CostRequest<'_>,
    database: &TimeZoneDatabase,
) -> Result<GenericCostResult, DashscopeCostError> {
    let breakdown = breakdown(request)?;
    let model_info = request.model_info;
    let tier = select_tier_for_input(&model_info.tiered_pricing, breakdown.total_input()?)?;
    let standard = match tier {
        Some(selected) => tier_rates(&model_info.record, selected)?,
        None => flat_rates(&model_info.record)?,
    };
    let rates = apply_off_peak_pricing(
        model_info.off_peak_pricing.as_ref(),
        request.current_time,
        standard,
        database,
    )?;
    let (prompt_cost, completion_cost) = bill(&breakdown, rates)?;
    Ok(GenericCostResult {
        prompt_cost,
        completion_cost,
    })
}

fn breakdown(request: &CostRequest<'_>) -> Result<Breakdown, DashscopeCostError> {
    let details = prompt_tokens_details(request.usage, request.prompt_extras)?;
    let completion_tokens = request.usage.completion_tokens;
    let completion_details = request.usage.completion_details.unwrap_or_default();
    let reasoning =
        parse_completion_details(completion_tokens, &completion_details)?.reasoning_tokens;
    Ok(Breakdown {
        text: request
            .usage
            .prompt_tokens
            .saturating_sub(details.cache_hit_tokens)
            .saturating_sub(details.cache_creation_tokens)
            .max(0),
        cached: details.cache_hit_tokens,
        cache_creation: details.cache_creation_tokens,
        completion: completion_tokens.saturating_sub(reasoning).max(0),
        reasoning,
    })
}

fn bill(
    breakdown: &Breakdown,
    rates: TokenRates,
) -> Result<(Decimal, Decimal), DashscopeCostError> {
    let text = product(breakdown.text, rates.input_rate)?;
    let cached = product(breakdown.cached, rates.cache_read_rate)?;
    let creation = product(breakdown.cache_creation, rates.cache_creation_rate)?;
    let prompt = text
        .checked_add(cached)
        .and_then(|total| total.checked_add(creation))
        .ok_or(DashscopeCostError::Overflow)?;
    let output = product(breakdown.completion, rates.output_rate)?;
    let reasoning = product(breakdown.reasoning, rates.billed_reasoning_rate())?;
    let completion = output
        .checked_add(reasoning)
        .ok_or(DashscopeCostError::Overflow)?;
    Ok((prompt, completion))
}

fn product(count: i64, rate: Decimal) -> Result<Decimal, DashscopeCostError> {
    Decimal::from(count)
        .checked_mul(rate)
        .ok_or(DashscopeCostError::Overflow)
}

fn flat_rates(model_info: &ModelInfo) -> Result<TokenRates, DashscopeCostError> {
    Ok(TokenRates {
        input_rate: coerce_cost_per_token(model_info.get(INPUT_COST))?,
        output_rate: coerce_cost_per_token(model_info.get(OUTPUT_COST))?,
        cache_read_rate: tier_rate(model_info.get(CACHE_READ_COST), model_info.get(INPUT_COST))?,
        cache_creation_rate: tier_rate(
            model_info.get(CACHE_CREATION_COST),
            model_info.get(INPUT_COST),
        )?,
        reasoning_rate: flat_reasoning_rate(model_info)?,
    })
}

fn flat_reasoning_rate(model_info: &ModelInfo) -> Result<Option<Decimal>, DashscopeCostError> {
    match model_info.get(REASONING_COST) {
        None | Some(Scalar::Null) => Ok(None),
        present => Ok(Some(coerce_cost_per_token(present)?)),
    }
}

fn tier_rates(model_info: &ModelInfo, tier: &Tier) -> Result<TokenRates, DashscopeCostError> {
    let flat = flat_rates(model_info)?;
    let declares_output = tier.output_token.is_some();
    let declares_reasoning = tier.output_reasoning_token.is_some();
    let reasoning_rate = if declares_reasoning {
        Some(tier_rate(tier.output_reasoning_token.as_ref(), None)?)
    } else if declares_output {
        None
    } else {
        flat.reasoning_rate
    };
    Ok(TokenRates {
        input_rate: tier_rate(tier.input_token.as_ref(), None)?,
        output_rate: if declares_output {
            tier_rate(tier.output_token.as_ref(), None)?
        } else {
            flat.output_rate
        },
        cache_read_rate: tier_rate(
            tier.cache_read_input_token_cost.as_ref(),
            tier.input_token.as_ref(),
        )?,
        cache_creation_rate: tier_rate(
            tier.cache_creation_token_cost.as_ref(),
            tier.input_token.as_ref(),
        )?,
        reasoning_rate,
    })
}

mod tests;
