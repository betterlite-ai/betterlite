use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde_json::Number;

use crate::decisions::Json;
use crate::tool_signals::error::ToolSignalError;
use crate::tool_signals::response::{OutputItemType, ToolBillingResponse};
use crate::tool_signals::text::{digit_magnitude, valid_underscored_digits};

// Only a typed Responses object counts its items; every other response bills one call on the per-call path.
pub fn count_web_search_calls(response: &ToolBillingResponse) -> Result<i64, ToolSignalError> {
    let ToolBillingResponse::Responses(view) = response else {
        return Ok(1);
    };
    if let Some(reported) = view
        .tool_usage
        .as_ref()
        .and_then(reported_web_search_requests)
    {
        return Ok(reported);
    }
    let items = view
        .output
        .iter()
        .filter(|item| item.item_type() == Some(OutputItemType::WebSearchCall.as_str()))
        .count();
    let count = i64::try_from(items).map_err(|_error| ToolSignalError::CounterOutOfRange)?;
    Ok(count.max(1))
}

// ResponsesToolUsage's non-negative web_search.num_requests; an invalid shape falls back to counting output items.
fn reported_web_search_requests(tool_usage: &Json) -> Option<i64> {
    let Json::Object(fields) = tool_usage else {
        return None;
    };
    let Json::Object(web_search) = fields.get("web_search")? else {
        return None;
    };
    web_search.get("num_requests").and_then(non_negative_int)
}

fn non_negative_int(value: &Json) -> Option<i64> {
    match pydantic_integer(value) {
        Some(count) if count >= 0 => Some(count),
        Some(_) | None => None,
    }
}

// Pydantic's lax int: a Boolean or an integral number counts, a numeric string parses, any other shape fails.
fn pydantic_integer(value: &Json) -> Option<i64> {
    match value {
        Json::Bool(flag) => Some(i64::from(*flag)),
        Json::Number(number) => pydantic_number(number),
        Json::Text(text) => pydantic_text(text),
        Json::Null | Json::List(_) | Json::Object(_) => None,
    }
}

fn pydantic_number(number: &Number) -> Option<i64> {
    let literal = number.to_string();
    if let Some(value) = number.as_i64() {
        return Some(value);
    }
    if !literal.contains(['.', 'e', 'E']) {
        return None;
    }
    let value: f64 = literal.parse().ok()?;
    if !value.is_finite() || value.fract() != 0.0_f64 {
        return None;
    }
    let decimal = Decimal::try_from(value).ok()?;
    if decimal <= Decimal::from(i64::MIN) || decimal >= Decimal::from(i64::MAX) {
        return None;
    }
    decimal.to_i64()
}

fn pydantic_text(text: &str) -> Option<i64> {
    let trimmed = text.trim();
    let (negative, digits) = trimmed.strip_prefix('-').map_or_else(
        || (false, trimmed.strip_prefix('+').unwrap_or(trimmed)),
        |rest| (true, rest),
    );
    let (integer_part, fraction) = match digits.split_once('.') {
        Some((integer, fraction)) => (integer, Some(fraction)),
        None => (digits, None),
    };
    if !valid_underscored_digits(integer_part) {
        return None;
    }
    if let Some(fraction_digits) = fraction
        && (fraction_digits.is_empty()
            || !fraction_digits.bytes().all(|digit| digit.is_ascii_digit())
            || fraction_digits.bytes().any(|digit| digit != b'0'))
    {
        return None;
    }
    let magnitude = digit_magnitude(integer_part)?;
    let signed = if negative {
        magnitude.checked_neg()?
    } else {
        magnitude
    };
    i64::try_from(signed).ok()
}

mod tests;
