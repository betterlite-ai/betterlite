use betterlite_openai::api::responses_objects::ResponseApiUsage;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde_json::{Map, Value};
use std::str::FromStr as _;

pub(super) fn parse(fields: &Map<String, Value>) -> Result<ResponseApiUsage, serde_json::Error> {
    let mut normalized = fields.clone();
    fill_detail_alias(
        &mut normalized,
        "input_tokens_details",
        "input_token_details",
    );
    fill_detail_alias(
        &mut normalized,
        "output_tokens_details",
        "output_token_details",
    );
    normalize_output_ints(&mut normalized);
    fill_total_tokens(&mut normalized);
    serde_json::from_value(Value::Object(normalized))
}

fn fill_detail_alias(fields: &mut Map<String, Value>, plural: &str, singular: &str) {
    if fields.get(plural).is_none_or(Value::is_null)
        && let Some(alias) = fields.get(singular).cloned()
    {
        fields.insert(plural.to_owned(), alias);
    }
}

pub(super) fn lax_integer(value: &Value) -> Result<Option<i64>, serde_json::Error> {
    match value {
        Value::Null => return Ok(None),
        Value::Bool(boolean) => return Ok(Some(i64::from(*boolean))),
        Value::Number(number) => {
            if let Some(integer) = number.as_i64() {
                return Ok(Some(integer));
            }
            if let Some(unsigned) = number.as_u64() {
                if let Ok(integer) = i64::try_from(unsigned) {
                    return Ok(Some(integer));
                }
                return serde_json::from_value::<i64>(value.clone()).map(Some);
            }
            if let Ok(decimal) = Decimal::from_str(&number.to_string())
                && decimal.fract().is_zero()
                && let Some(integer) = decimal.to_i64()
            {
                return Ok(Some(integer));
            }
        }
        Value::String(string) => {
            let trimmed = string.trim();
            let (integer_part, fractional) = trimmed.split_once('.').unwrap_or((trimmed, ""));
            let fractional_is_zero =
                !fractional.is_empty() && fractional.bytes().all(|byte| byte == b'0');
            if (!trimmed.contains('.') || fractional_is_zero)
                && let Some(integer_text) = strip_digit_separators(integer_part)
                && let Ok(parsed) = integer_text.parse::<i64>()
            {
                return Ok(Some(parsed));
            }
        }
        Value::Array(_) | Value::Object(_) => {}
    }
    serde_json::from_value::<i64>(value.clone()).map(Some)
}

fn strip_digit_separators(value: &str) -> Option<String> {
    let mut normalized = String::with_capacity(value.len());
    let mut previous_digit = false;
    let mut characters = value.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '_' && previous_digit && characters.peek().is_some_and(char::is_ascii_digit)
        {
            previous_digit = false;
        } else if character == '_' {
            return None;
        } else {
            normalized.push(character);
            previous_digit = character.is_ascii_digit();
        }
    }
    Some(normalized)
}

fn normalize_output_ints(fields: &mut Map<String, Value>) {
    let Some(Value::Object(details)) = fields.get_mut("output_tokens_details") else {
        return;
    };
    for name in ["reasoning_tokens", "audio_tokens", "text_tokens"] {
        if let Some(value) = details.get(name)
            && let Ok(Some(integer)) = lax_integer(value)
        {
            details.insert(name.to_owned(), Value::from(integer));
        }
    }
}

fn fill_total_tokens(fields: &mut Map<String, Value>) {
    if fields
        .get("total_tokens")
        .is_some_and(|value| !value.is_null())
    {
        return;
    }
    let input = fields
        .get("input_tokens")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let output = fields
        .get("output_tokens")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    fields.insert(
        "total_tokens".to_owned(),
        Value::from(input.saturating_add(output)),
    );
}
