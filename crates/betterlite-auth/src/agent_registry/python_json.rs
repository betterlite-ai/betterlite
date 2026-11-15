use std::fmt::Write as _;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

mod tests;

#[derive(Debug, thiserror::Error)]
pub enum JsonEncodingError {
    #[error("config entry holds an integer outside the i64 and u64 range")]
    IntegerOutOfRange,
    #[error("config entry holds a number that is not an IEEE 754 double")]
    UnrepresentableNumber,
}

pub(crate) fn dumps(value: &Json) -> Result<String, JsonEncodingError> {
    let mut out = String::new();
    push_value(&mut out, value)?;
    Ok(out)
}

fn push_value(out: &mut String, value: &Json) -> Result<(), JsonEncodingError> {
    match value {
        Json::Null => out.push_str("null"),
        Json::Bool(true) => out.push_str("true"),
        Json::Bool(false) => out.push_str("false"),
        Json::Number(number) => out.push_str(&number_text(number)?),
        Json::Text(text) => push_text(out, text),
        Json::List(items) => push_list(out, items)?,
        Json::Object(fields) => push_object(out, fields)?,
    }
    Ok(())
}

fn push_list(out: &mut String, items: &[Json]) -> Result<(), JsonEncodingError> {
    out.push('[');
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        push_value(out, item)?;
    }
    out.push(']');
    Ok(())
}

fn push_object(out: &mut String, fields: &IndexMap<String, Json>) -> Result<(), JsonEncodingError> {
    let mut entries: Vec<(&String, &Json)> = fields.iter().collect();
    entries.sort_unstable_by(|(left, _), (right, _)| left.cmp(right));
    out.push('{');
    for (index, (key, value)) in entries.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        push_text(out, key);
        out.push_str(": ");
        push_value(out, value)?;
    }
    out.push('}');
    Ok(())
}

fn number_text(number: &Number) -> Result<String, JsonEncodingError> {
    if let Some(value) = number.as_i64() {
        return Ok(value.to_string());
    }
    if let Some(value) = number.as_u64() {
        return Ok(value.to_string());
    }
    let stored = number.to_string();
    if !stored.contains(['.', 'e', 'E']) {
        return Err(JsonEncodingError::IntegerOutOfRange);
    }
    let Some(value) = number.as_f64() else {
        return Err(JsonEncodingError::UnrepresentableNumber);
    };
    Ok(float_text(value))
}

pub(crate) fn float_text(value: f64) -> String {
    if value.is_nan() {
        return String::from("NaN");
    }
    if value.is_infinite() {
        if value.is_sign_negative() {
            return String::from("-Infinity");
        }
        return String::from("Infinity");
    }
    let rendered = format!("{value:e}");
    let Some((mantissa, exponent)) = rendered.split_once('e') else {
        return rendered;
    };
    let Ok(exponent) = exponent.parse::<i32>() else {
        return rendered;
    };
    let (negative, mantissa) = match mantissa.strip_prefix('-') {
        Some(stripped) => (true, stripped),
        None => (false, mantissa),
    };
    let digits: String = mantissa
        .chars()
        .filter(|character| *character != '.')
        .collect();
    let mut out = String::new();
    if negative {
        out.push('-');
    }
    let decpt = exponent.saturating_add(1);
    if decpt <= -4 || decpt > 16 {
        push_exponential(&mut out, &digits, decpt.saturating_sub(1));
    } else {
        push_fixed(&mut out, &digits, decpt);
    }
    out
}

fn push_fixed(out: &mut String, digits: &str, decpt: i32) {
    if decpt <= 0 {
        out.push_str("0.");
        push_zeros(out, decpt.saturating_neg());
        out.push_str(digits);
        return;
    }
    // the fixed branch only sees a decimal point at 1..=16, so the conversion cannot fail
    let split = usize::try_from(decpt).unwrap_or(digits.len());
    if split >= digits.len() {
        out.push_str(digits);
        push_zeros(out, split.saturating_sub(digits.len()));
        out.push_str(".0");
        return;
    }
    for (index, character) in digits.chars().enumerate() {
        if index == split {
            out.push('.');
        }
        out.push(character);
    }
}

fn push_exponential(out: &mut String, digits: &str, exponent: i32) {
    let mut characters = digits.chars();
    let Some(first) = characters.next() else {
        return;
    };
    out.push(first);
    let rest = characters.as_str();
    if !rest.is_empty() {
        out.push('.');
        out.push_str(rest);
    }
    out.push('e');
    if exponent.is_negative() {
        out.push('-');
    } else {
        out.push('+');
    }
    let magnitude = exponent.unsigned_abs();
    write!(out, "{magnitude:02}").unwrap_or(());
}

fn push_zeros(out: &mut String, count: i32) {
    for _ in 0..count {
        out.push('0');
    }
}

fn push_text(out: &mut String, text: &str) {
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            ' '..='~' => out.push(character),
            escaped => push_escaped(out, escaped),
        }
    }
    out.push('"');
}

fn push_escaped(out: &mut String, character: char) {
    let code = u32::from(character);
    if code <= 0xffff {
        write!(out, "\\u{code:04x}").unwrap_or(());
        return;
    }
    let offset = code.saturating_sub(0x10000);
    let high = offset.checked_div(0x400).unwrap_or(0);
    let low = offset.checked_rem(0x400).unwrap_or(0);
    let high_unit = 0xd800_u32.saturating_add(high);
    write!(out, "\\u{high_unit:04x}").unwrap_or(());
    let low_unit = 0xdc00_u32.saturating_add(low);
    write!(out, "\\u{low_unit:04x}").unwrap_or(());
}
