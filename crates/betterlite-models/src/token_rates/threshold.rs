use std::cmp::Reverse;

use rust_decimal::Decimal;

use super::base_cost::BaseRates;
use super::rates::{
    ServiceTier, cost_per_unit, matches_float_literal_grammar, service_tier_cost_key,
};
use super::{ModelInfo, RequestContext, TokenRateError, Usage};
use crate::price_fields::decimal_lexeme::parse_decimal_lexeme;
use crate::tiered_pricing::Scalar;

const INPUT_THRESHOLD_PREFIX: &str = "input_cost_per_token_above_";

// A text that is not itself a valid float literal is a typed error, matching the sort it feeds.
pub(super) fn parse_token_threshold(text: &str, key: &str) -> Result<Decimal, TokenRateError> {
    let has_k = text.contains('k');
    let stripped: String = text.chars().filter(|&character| character != 'k').collect();
    let trimmed = stripped.trim();
    if !matches_float_literal_grammar(trimmed) {
        return Err(TokenRateError::InvalidThreshold {
            key: key.to_owned(),
        });
    }
    let value =
        parse_decimal_lexeme(&trimmed.replace('_', "")).map_err(|source| TokenRateError::Rate {
            key: key.to_owned(),
            source,
        })?;
    if has_k {
        value
            .checked_mul(Decimal::from(1000_u16))
            .ok_or(TokenRateError::Overflow)
    } else {
        Ok(value)
    }
}

pub(super) fn prompt_exceeds(prompt_tokens: i64, limit: Decimal, inclusive: bool) -> bool {
    let tokens = Decimal::from(prompt_tokens);
    tokens > limit || (inclusive && tokens == limit)
}

pub(super) fn crossed_input_threshold(
    model: &ModelInfo,
    usage: Usage,
    context: &RequestContext<'_>,
    service_tier: Option<&str>,
) -> Result<Option<String>, TokenRateError> {
    let tier_suffix = service_tier_cost_key("", service_tier);
    let inclusive = context.provider == Some("xai");
    let mut parsed = Vec::new();
    for key in model.keys() {
        if !is_threshold_candidate(key, &tier_suffix) {
            continue;
        }
        let text = above_text(key);
        let limit = parse_token_threshold(text, key)?;
        parsed.push((limit, key.clone(), text.to_owned()));
    }
    parsed.sort_by_key(|(limit, _, _)| Reverse(*limit));
    for (limit, key, text) in parsed {
        if matches!(model.get(&key), None | Some(Scalar::Null)) {
            continue;
        }
        if prompt_exceeds(usage.prompt_tokens, limit, inclusive) {
            return Ok(Some(text));
        }
    }
    Ok(None)
}

fn is_threshold_candidate(key: &str, tier_suffix: &str) -> bool {
    if !key.starts_with(INPUT_THRESHOLD_PREFIX) {
        return false;
    }
    let is_non_standard = ServiceTier::ALL
        .into_iter()
        .map(ServiceTier::suffix)
        .chain(["_batches"])
        .any(|suffix| key.ends_with(suffix));
    !is_non_standard || (!tier_suffix.is_empty() && key.ends_with(tier_suffix))
}

// A key without `_tokens` at all is used in full, matching the source's lenient split.
fn above_text(key: &str) -> &str {
    let rest = key.strip_prefix(INPUT_THRESHOLD_PREFIX).unwrap_or(key);
    rest.split("_tokens").next().unwrap_or(rest)
}

pub(super) fn apply_threshold_overrides(
    model: &ModelInfo,
    service_tier: Option<&str>,
    threshold_text: &str,
    rates: &BaseRates,
) -> Result<BaseRates, TokenRateError> {
    let keyed = |prefix: &str| {
        service_tier_cost_key(
            &format!("{prefix}_above_{threshold_text}_tokens"),
            service_tier,
        )
    };
    let prompt = cost_per_unit(model, &keyed("input_cost_per_token"), Some(rates.prompt))?
        .unwrap_or(rates.prompt);
    let completion = cost_per_unit(
        model,
        &keyed("output_cost_per_token"),
        Some(rates.completion),
    )?
    .unwrap_or(rates.completion);
    let cache_creation = cost_per_unit(
        model,
        &keyed("cache_creation_input_token_cost"),
        rates.cache_creation,
    )?;
    let cache_creation_above_1hr = cost_per_unit(
        model,
        &keyed("cache_creation_input_token_cost_above_1hr"),
        rates.cache_creation_above_1hr,
    )?;
    let cache_read = cost_per_unit(
        model,
        &keyed("cache_read_input_token_cost"),
        rates.cache_read,
    )?;
    Ok(BaseRates {
        prompt,
        completion,
        cache_creation,
        cache_creation_above_1hr,
        cache_read,
    })
}
