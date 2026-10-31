use crate::price_fields::decimal_lexeme::parse_decimal_lexeme;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

mod tests;

pub type ModelInfo = BTreeMap<String, Value>;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Usage {
    pub prompt_tokens: i64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct EffectiveRates {
    pub input_rate: Decimal,
    pub output_rate: Decimal,
    pub cache_read_rate: Decimal,
    pub cache_creation_rate: Decimal,
    pub cache_creation_above_1hr_rate: Decimal,
    pub reasoning_rate: Decimal,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct BatchCostRates {
    pub input: Option<Decimal>,
    pub output: Option<Decimal>,
    pub cache_read: Option<Decimal>,
    pub cache_creation: Option<Decimal>,
}

#[derive(Debug, thiserror::Error)]
pub enum TokenRateError {
    #[error("invalid token threshold {key}")]
    InvalidThreshold { key: String },
    #[error("invalid exact rate for {key}: {source}")]
    Rate {
        key: String,
        #[source]
        source: rust_decimal::Error,
    },
    #[error("rate multiplication overflow")]
    Overflow,
}

const SERVICE_SUFFIXES: &[&str] = &["_ultrafast", "_balanced", "_priority", "_flex"];
const BATCH_PREFIXES: &[&str] = &[
    "input_cost_per_token",
    "output_cost_per_token",
    "cache_read_input_token_cost",
    "cache_creation_input_token_cost",
];

pub fn service_tier_cost_key(base: &str, tier: Option<&str>) -> String {
    let Some(tier) = tier.map(str::to_ascii_lowercase) else {
        return base.to_owned();
    };
    let suffix = match tier.as_str() {
        "fast" | "priority" => "priority",
        "flex" => "flex",
        "balanced" => "balanced",
        "ultrafast" => "ultrafast",
        _ => return base.to_owned(),
    };
    format!("{base}_{suffix}")
}

fn decimal_value(
    key: &str,
    value: Option<&Value>,
    default: Option<Decimal>,
) -> Result<Option<Decimal>, TokenRateError> {
    let Some(value) = value else {
        return Ok(default);
    };
    match value {
        Value::Null => Ok(default),
        Value::Bool(flag) => Ok(Some(Decimal::from(u8::from(*flag)))),
        Value::Number(number) => parse_rate(key, &number.to_string()).map(Some),
        Value::String(text) => match parse_decimal_lexeme(text.trim()) {
            Ok(rate) => Ok(Some(rate)),
            Err(_) => Ok(default),
        },
        Value::Array(_) | Value::Object(_) => Ok(default),
    }
}

fn parse_rate(key: &str, text: &str) -> Result<Decimal, TokenRateError> {
    let result = parse_decimal_lexeme(text).map_err(|error| TokenRateError::Rate {
        key: key.to_owned(),
        source: match error {
            crate::price_fields::decimal_lexeme::DecimalLexemeError::Decimal(source) => source,
            _ => rust_decimal::Error::ExceedsMaximumPossibleValue,
        },
    });
    result
}

fn cost_per_unit(
    model: &ModelInfo,
    key: &str,
    default: Option<Decimal>,
) -> Result<Option<Decimal>, TokenRateError> {
    match model.get(key) {
        Some(value @ Value::String(_))
        | Some(value @ Value::Number(_))
        | Some(value @ Value::Bool(_)) => decimal_value(key, Some(value), default),
        Some(Value::Null) | None => {
            for suffix in SERVICE_SUFFIXES {
                if key.contains(suffix) {
                    let base = key.replacen(suffix, "", 1);
                    return decimal_value(&base, model.get(&base), default);
                }
            }
            Ok(default)
        }
        Some(Value::Array(_)) | Some(Value::Object(_)) => Ok(default),
    }
}

fn rate(model: &ModelInfo, key: &str, default: Decimal) -> Result<Decimal, TokenRateError> {
    let result = cost_per_unit(model, key, Some(default)).map(|value| value.unwrap_or(default));
    result
}

fn threshold(text: &str, key: &str) -> Result<Decimal, TokenRateError> {
    let Some(number) = text.strip_suffix('k') else {
        if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(TokenRateError::InvalidThreshold {
                key: key.to_owned(),
            });
        }
        return parse_rate(key, text);
    };
    if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(TokenRateError::InvalidThreshold {
            key: key.to_owned(),
        });
    }
    parse_rate(key, number)?
        .checked_mul(Decimal::from(1000u16))
        .ok_or(TokenRateError::Overflow)
}

fn above_key(key: &str) -> Option<(&str, &str)> {
    let (prefix, rest) = key.split_once("_above_")?;
    let threshold = rest.strip_suffix("_tokens")?;
    Some((prefix, threshold))
}

fn crossed_threshold(
    model: &ModelInfo,
    usage: Usage,
    tier: Option<&str>,
    inclusive: bool,
) -> Result<Option<String>, TokenRateError> {
    let suffix = service_tier_cost_key("", tier);
    let mut keys = Vec::new();
    for key in model.keys() {
        let Some((prefix, text)) = above_key(key) else {
            continue;
        };
        if prefix != "input_cost_per_token"
            || (key.ends_with("_batches")
                || (!suffix.is_empty() && !key.ends_with(&suffix))
                    && SERVICE_SUFFIXES.iter().any(|item| key.ends_with(item)))
        {
            continue;
        }
        let parsed = threshold(text, key)?;
        keys.push((parsed, key.clone(), text.to_owned()));
    }
    keys.sort_by(|left, right| right.0.cmp(&left.0));
    Ok(keys
        .into_iter()
        .find(|(limit, _, _)| {
            Decimal::from(usage.prompt_tokens) > *limit
                || (inclusive && Decimal::from(usage.prompt_tokens) == *limit)
        })
        .map(|(_, _, text)| text))
}

fn tiered_base(model: &ModelInfo, usage: Usage) -> Result<Option<EffectiveRates>, TokenRateError> {
    let Some(Value::Array(tiers)) = model.get("tiered_pricing") else {
        return Ok(None);
    };
    if tiers.is_empty() || usage.prompt_tokens <= 0 {
        return Ok(None);
    }
    let mut selected = None;
    for tier in tiers {
        let Some(object) = tier.as_object() else {
            continue;
        };
        let Some(Value::Array(range)) = object.get("range") else {
            continue;
        };
        let Some(lower) = range.first().and_then(Value::as_i64) else {
            continue;
        };
        let upper = range.get(1).and_then(Value::as_i64).unwrap_or(i64::MAX);
        if usage.prompt_tokens > lower && usage.prompt_tokens <= upper {
            selected = Some(object);
            break;
        }
    }
    let Some(tier) = selected else {
        return Ok(None);
    };
    if !tier.contains_key("input_cost_per_token") {
        return Ok(None);
    }
    let input = decimal_value(
        "input_cost_per_token",
        tier.get("input_cost_per_token"),
        Some(Decimal::ZERO),
    )?
    .unwrap_or_default();
    let output = decimal_value(
        "output_cost_per_token",
        tier.get("output_cost_per_token"),
        cost_per_unit(model, "output_cost_per_token", Some(Decimal::ZERO))?,
    )?
    .unwrap_or_default();
    let cache = decimal_value(
        "cache_creation_input_token_cost",
        tier.get("cache_creation_input_token_cost"),
        Some(input),
    )?
    .unwrap_or(input);
    let read = decimal_value(
        "cache_read_input_token_cost",
        tier.get("cache_read_input_token_cost"),
        Some(input),
    )?
    .unwrap_or(input);
    let hour = decimal_value(
        "cache_creation_input_token_cost_above_1hr",
        tier.get("cache_creation_input_token_cost_above_1hr"),
        Some(cache),
    )?
    .unwrap_or(cache);
    let reasoning = decimal_value(
        "output_cost_per_reasoning_token",
        tier.get("output_cost_per_reasoning_token"),
        Some(output),
    )?
    .unwrap_or(output);
    Ok(Some(EffectiveRates {
        input_rate: input,
        output_rate: output,
        cache_read_rate: read,
        cache_creation_rate: cache,
        cache_creation_above_1hr_rate: hour,
        reasoning_rate: reasoning,
    }))
}

pub fn get_token_base_cost(
    model: &ModelInfo,
    usage: Usage,
    service_tier: Option<&str>,
    provider: Option<&str>,
) -> Result<EffectiveRates, TokenRateError> {
    if let Some(rates) = tiered_base(model, usage)? {
        return Ok(rates);
    }
    let inclusive = provider == Some("xai");
    let input_key = service_tier_cost_key("input_cost_per_token", service_tier);
    let output_key = service_tier_cost_key("output_cost_per_token", service_tier);
    let cache_key = service_tier_cost_key("cache_creation_input_token_cost", service_tier);
    let read_key = service_tier_cost_key("cache_read_input_token_cost", service_tier);
    let input = rate(model, &input_key, Decimal::ZERO)?;
    let mut output = rate(model, &output_key, Decimal::ZERO)?;
    if output.is_zero() {
        output = rate(model, "output_cost_per_image_token", output)?;
    }
    let threshold = crossed_threshold(model, usage, service_tier, inclusive)?;
    let mut cache = cost_per_unit(model, &cache_key, None)?;
    let mut read = cost_per_unit(model, &read_key, None)?;
    let hour = cost_per_unit(model, "cache_creation_input_token_cost_above_1hr", None)?;
    if let Some(text) = threshold {
        let keys = [
            ("input_cost_per_token", input_key.as_str()),
            ("output_cost_per_token", output_key.as_str()),
            ("cache_creation_input_token_cost", cache_key.as_str()),
            ("cache_read_input_token_cost", read_key.as_str()),
        ];
        let suffix = service_tier
            .map(|_| service_tier_cost_key("", service_tier))
            .unwrap_or_default();
        let base = |prefix: &str| format!("{prefix}_above_{text}_tokens{suffix}");
        let input = cost_per_unit(model, &base(keys[0].0), Some(input))?.unwrap_or(input);
        output = cost_per_unit(model, &base(keys[1].0), Some(output))?.unwrap_or(output);
        cache = cost_per_unit(model, &base(keys[2].0), cache)?.or(cache);
        read = cost_per_unit(model, &base(keys[3].0), read)?.or(read);
        return finish(model, input, output, cache, read, hour, service_tier);
    }
    finish(model, input, output, cache, read, hour, service_tier)
}

fn finish(
    model: &ModelInfo,
    input: Decimal,
    output: Decimal,
    cache: Option<Decimal>,
    read: Option<Decimal>,
    hour: Option<Decimal>,
    tier: Option<&str>,
) -> Result<EffectiveRates, TokenRateError> {
    let cache = cache.unwrap_or(input);
    let read = read.unwrap_or(input);
    let hour = hour.unwrap_or(cache);
    let reasoning_key = service_tier_cost_key("output_cost_per_reasoning_token", tier);
    let reasoning = cost_per_unit(model, &reasoning_key, None)?
        .or_else(|| {
            cost_per_unit(model, "output_cost_per_reasoning_token", None)
                .ok()
                .flatten()
        })
        .unwrap_or(output);
    Ok(EffectiveRates {
        input_rate: input,
        output_rate: output,
        cache_read_rate: read,
        cache_creation_rate: cache,
        cache_creation_above_1hr_rate: hour,
        reasoning_rate: reasoning,
    })
}

pub fn calculate_cost_component(
    model: &ModelInfo,
    key: &str,
    usage: Option<Decimal>,
) -> Result<Decimal, TokenRateError> {
    let Some(usage) = usage else {
        return Ok(Decimal::ZERO);
    };
    if usage <= Decimal::ZERO {
        return Ok(Decimal::ZERO);
    }
    let rate = cost_per_unit(model, key, None)?.unwrap_or(Decimal::ZERO);
    usage.checked_mul(rate).ok_or(TokenRateError::Overflow)
}

pub fn deployment_pricing(model: Option<&ModelInfo>) -> Result<Option<ModelInfo>, TokenRateError> {
    let Some(model) = model else { return Ok(None) };
    let keys = [
        "input_cost_per_token",
        "output_cost_per_token",
        "cache_read_input_token_cost",
        "cache_creation_input_token_cost",
        "cache_creation_input_token_cost_above_1hr",
    ];
    let mut pricing = ModelInfo::new();
    for key in keys {
        if let Some(rate) = cost_per_unit(model, key, None)? {
            pricing.insert(key.to_owned(), Value::String(rate.normalize().to_string()));
        }
    }
    Ok((!pricing.is_empty()).then_some(pricing))
}

pub fn get_batch_cost_rates(
    model: &ModelInfo,
    usage: Usage,
    provider: Option<&str>,
) -> Result<BatchCostRates, TokenRateError> {
    let inclusive = provider == Some("xai");
    let mut result = BatchCostRates::default();
    for (prefix, target) in [
        (BATCH_PREFIXES[0], &mut result.input),
        (BATCH_PREFIXES[1], &mut result.output),
        (BATCH_PREFIXES[2], &mut result.cache_read),
        (BATCH_PREFIXES[3], &mut result.cache_creation),
    ] {
        let mut selected: Option<(Decimal, String)> = None;
        for key in model.keys() {
            let Some((found, text)) = above_key(key) else {
                continue;
            };
            if found != prefix || !key.ends_with("_batches") {
                continue;
            };
            let limit = threshold(text, key)?;
            if (Decimal::from(usage.prompt_tokens) > limit
                || (inclusive && Decimal::from(usage.prompt_tokens) == limit))
                && selected.as_ref().is_none_or(|old| limit > old.0)
            {
                selected = Some((limit, text.to_owned()));
            }
        }
        let flat = cost_per_unit(model, &format!("{prefix}_batches"), None)?;
        *target = match selected {
            Some((_, text)) => cost_per_unit(
                model,
                &format!("{prefix}_above_{text}_tokens_batches"),
                flat,
            )?,
            None => flat,
        };
    }
    Ok(result)
}
