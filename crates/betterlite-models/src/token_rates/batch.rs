use std::cmp::Reverse;

use rust_decimal::Decimal;

use super::rates::parse_text_rate;
use super::threshold::prompt_exceeds;
use super::{BatchCostRates, ModelInfo, TokenRateError, Usage};
use crate::tiered_pricing::Scalar;

const BATCH_PREFIXES: [&str; 4] = [
    "input_cost_per_token",
    "output_cost_per_token",
    "cache_read_input_token_cost",
    "cache_creation_input_token_cost",
];

pub fn get_batch_cost_rates(
    model: &ModelInfo,
    usage: Usage,
    provider: Option<&str>,
) -> Result<BatchCostRates, TokenRateError> {
    let inclusive = provider == Some("xai");
    Ok(BatchCostRates {
        input: batch_rate_for_prefix(model, BATCH_PREFIXES[0], usage, inclusive)?,
        output: batch_rate_for_prefix(model, BATCH_PREFIXES[1], usage, inclusive)?,
        cache_read: batch_rate_for_prefix(model, BATCH_PREFIXES[2], usage, inclusive)?,
        cache_creation: batch_rate_for_prefix(model, BATCH_PREFIXES[3], usage, inclusive)?,
    })
}

// A direct value parse only: batch rates never take the service-tier suffix fallback.
fn batch_rate(model: &ModelInfo, key: &str) -> Result<Option<Decimal>, TokenRateError> {
    match model.get(key) {
        Some(Scalar::Number(number)) => {
            super::rates::parse_rate(key, &number.to_string()).map(Some)
        }
        Some(Scalar::Bool(flag)) => Ok(Some(Decimal::from(u8::from(*flag)))),
        Some(Scalar::Text(text)) => parse_text_rate(key, text.trim(), None),
        Some(Scalar::Null | Scalar::Array(_) | Scalar::Object(_)) | None => Ok(None),
    }
}

// A malformed tier rate falls back to the flat rate, never to a lower threshold.
fn batch_rate_for_prefix(
    model: &ModelInfo,
    prefix: &str,
    usage: Usage,
    inclusive: bool,
) -> Result<Option<Decimal>, TokenRateError> {
    let flat_key = format!("{prefix}_batches");
    let Some(threshold) = crossed_batch_tier(model, prefix, usage, inclusive)? else {
        return batch_rate(model, &flat_key);
    };
    let tier_key = format!("{prefix}_above_{threshold}_tokens_batches");
    batch_rate(model, &tier_key)?
        .map_or_else(|| batch_rate(model, &flat_key), |value| Ok(Some(value)))
}

fn crossed_batch_tier(
    model: &ModelInfo,
    prefix: &str,
    usage: Usage,
    inclusive: bool,
) -> Result<Option<String>, TokenRateError> {
    let mut thresholds = Vec::new();
    for (key, value) in model {
        if matches!(value, Scalar::Null) {
            continue;
        }
        let Some(text) = batch_threshold_text(key, prefix) else {
            continue;
        };
        let limit = batch_threshold_value(text)?;
        thresholds.push((limit, text.to_owned()));
    }
    thresholds.sort_by_key(|(limit, _)| Reverse(*limit));
    Ok(thresholds
        .into_iter()
        .find(|(limit, _)| prompt_exceeds(usage.prompt_tokens, *limit, inclusive))
        .map(|(_, text)| text))
}

// `^{prefix}_above_(\d+k?)_tokens_batches$`, anchored both ends; a non-match is no key at all.
fn batch_threshold_text<'key>(key: &'key str, prefix: &str) -> Option<&'key str> {
    let rest = key.strip_prefix(prefix)?.strip_prefix("_above_")?;
    let text = rest.strip_suffix("_tokens_batches")?;
    let digits = text.strip_suffix('k').unwrap_or(text);
    (!digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())).then_some(text)
}

fn batch_threshold_value(text: &str) -> Result<Decimal, TokenRateError> {
    let has_k = text.ends_with('k');
    let digits = text.strip_suffix('k').unwrap_or(text);
    let value = digits
        .parse::<u64>()
        .map(Decimal::from)
        .map_err(|_error| TokenRateError::Overflow)?;
    if has_k {
        value
            .checked_mul(Decimal::from(1000_u16))
            .ok_or(TokenRateError::Overflow)
    } else {
        Ok(value)
    }
}
