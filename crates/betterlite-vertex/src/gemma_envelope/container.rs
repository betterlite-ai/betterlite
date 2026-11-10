use betterlite_models::decisions::Json;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde_json::Number;

pub const MIN_CODE: u16 = 400;
pub const MAX_CODE: u16 = 599;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VertexGemmaContainerError {
    pub code: u16,
    pub message: String,
}

// LiteLLM's pydantic record ignores unknown keys, requires object="error" and a string message, and coerces the code
#[must_use]
pub fn parse_gemma_container_error(predictions: &Json) -> Option<VertexGemmaContainerError> {
    let Json::Object(fields) = predictions else {
        return None;
    };
    let Some(Json::Text(object)) = fields.get("object") else {
        return None;
    };
    if object != "error" {
        return None;
    }
    let Some(Json::Text(message)) = fields.get("message") else {
        return None;
    };
    let code = code_value(fields.get("code")?)?;
    Some(VertexGemmaContainerError {
        code,
        message: message.clone(),
    })
}

fn code_value(value: &Json) -> Option<u16> {
    match value {
        Json::Number(number) => number_code(number),
        Json::Text(text) => text_code(text),
        Json::Null | Json::Bool(_) | Json::List(_) | Json::Object(_) => None,
    }
}

fn number_code(number: &Number) -> Option<u16> {
    if let Some(integer) = number.as_i64() {
        return u16::try_from(integer)
            .ok()
            .filter(|code| (MIN_CODE..=MAX_CODE).contains(code));
    }
    let decimal = Decimal::from_f64_retain(number.as_f64()?)?;
    if decimal.fract() != Decimal::ZERO {
        return None;
    }
    decimal
        .to_u16()
        .filter(|code| (MIN_CODE..=MAX_CODE).contains(code))
}

fn text_code(text: &str) -> Option<u16> {
    let trimmed = text.trim();
    let (negative, unsigned) = trimmed.strip_prefix('-').map_or_else(
        || {
            trimmed
                .strip_prefix('+')
                .map_or((false, trimmed), |rest| (false, rest))
        },
        |rest| (true, rest),
    );
    let (whole, fraction) = match unsigned.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (unsigned, None),
    };
    let value = parse_digits(whole)?;
    if let Some(rest) = fraction
        && (rest.is_empty() || !rest.bytes().all(|byte| byte == b'0'))
    {
        return None;
    }
    let signed = if negative {
        value.checked_neg()?
    } else {
        value
    };
    u16::try_from(signed)
        .ok()
        .filter(|code| (MIN_CODE..=MAX_CODE).contains(code))
}

// pydantic reads the string the way Python's int() does: ASCII digits, with single underscores between them
fn parse_digits(text: &str) -> Option<i64> {
    if text.is_empty() || text.starts_with('_') || text.ends_with('_') || text.contains("__") {
        return None;
    }
    let mut value: i64 = 0;
    for byte in text.bytes() {
        if byte == b'_' {
            continue;
        }
        if !byte.is_ascii_digit() {
            return None;
        }
        let digit = u32::from(byte).checked_sub(u32::from(b'0'))?;
        value = value.checked_mul(10)?.checked_add(i64::from(digit))?;
    }
    Some(value)
}
