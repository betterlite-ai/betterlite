use std::str::FromStr as _;

use betterlite_models::decisions::Json;
use betterlite_prometheus::collector_factory::collector::MetricValue;
use indexmap::IndexMap;
use rust_decimal::Decimal;

#[must_use]
pub fn object(value: Option<&Json>) -> Option<&IndexMap<String, Json>> {
    match value {
        Some(Json::Object(fields)) => Some(fields),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => None,
    }
}

#[must_use]
pub fn integer(value: Option<&Json>) -> Option<i64> {
    let Json::Number(number) = value? else {
        return None;
    };
    number.as_i64()
}

#[must_use]
pub fn positive_value(value: &Json) -> Option<MetricValue> {
    let Json::Number(number) = value else {
        return None;
    };
    let text = number.to_string();
    let parsed = if text.contains(['e', 'E']) {
        Decimal::from_scientific(&text)
    } else {
        Decimal::from_str(&text)
    };
    if let Ok(amount) = parsed {
        return (amount > Decimal::ZERO).then_some(MetricValue::Finite(amount));
    }
    let magnitude = text.parse::<f64>().ok()?;
    (magnitude > 0.0_f64).then_some(MetricValue::PositiveInfinity)
}

#[must_use]
pub fn resolve_cache_write_tokens(prompt_details: Option<&Json>) -> Option<i64> {
    let details = object(prompt_details)?;
    let write = integer(details.get("cache_write_tokens"));
    write.or_else(|| integer(details.get("cache_creation_tokens")))
}

#[must_use]
pub fn resolve_provider_cache_tokens(usage: &IndexMap<String, Json>) -> (i64, i64) {
    let read_field = usage.get("cache_read_input_tokens");
    let write_field = usage.get("cache_creation_input_tokens");
    let prompt_details = usage.get("prompt_tokens_details");
    let mut read = integer(read_field).unwrap_or(0);
    let mut creation = integer(write_field).unwrap_or(0);
    if read_field.is_none_or(|value| matches!(value, Json::Null))
        && let Some(cached) =
            object(prompt_details).and_then(|details| integer(details.get("cached_tokens")))
    {
        read = cached;
    }
    if write_field.is_none_or(|value| matches!(value, Json::Null))
        && let Some(written) = resolve_cache_write_tokens(prompt_details)
    {
        creation = written;
    }
    (read, creation)
}
