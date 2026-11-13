use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde_json::Number;

use crate::decisions::Json;
use crate::tool_signals::error::{QuantityField, ToolSignalError};
use crate::tool_signals::text::{digit_magnitude, is_float_literal, valid_underscored_digits};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FileSearchParams {
    pub storage_gb: Option<Decimal>,
    pub days: Option<Decimal>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TokenCounts {
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
}

// file_search may arrive as any object; only a mapping contributes storage_gb and days, each converted on its own.
pub fn extract_file_search_params(
    file_search_usage: Option<&Json>,
) -> Result<FileSearchParams, ToolSignalError> {
    let Some(Json::Object(fields)) = file_search_usage else {
        return Ok(FileSearchParams::default());
    };
    let storage_gb = match fields.get("storage_gb") {
        Some(value) => quantity_from_json(value, QuantityField::StorageGb)?,
        None => None,
    };
    let days = match fields.get("days") {
        Some(value) => quantity_from_json(value, QuantityField::Days)?,
        None => None,
    };
    Ok(FileSearchParams { storage_gb, days })
}

pub fn extract_token_counts(
    computer_use_usage: Option<&Json>,
) -> Result<TokenCounts, ToolSignalError> {
    let Some(Json::Object(fields)) = computer_use_usage else {
        return Ok(TokenCounts::default());
    };
    let input_tokens = match fields.get("input_tokens") {
        Some(value) => safe_convert_to_int(value)?,
        None => None,
    };
    let output_tokens = match fields.get("output_tokens") {
        Some(value) => safe_convert_to_int(value)?,
        None => None,
    };
    Ok(TokenCounts {
        input_tokens,
        output_tokens,
    })
}

// int(): integers, Booleans, truncating finite floats and numeric strings convert; anything else is None.
pub fn safe_convert_to_int(value: &Json) -> Result<Option<i64>, ToolSignalError> {
    match value {
        Json::Bool(flag) => Ok(Some(i64::from(*flag))),
        Json::Number(number) => int_number(number),
        Json::Text(text) => int_text(text),
        Json::Null | Json::List(_) | Json::Object(_) => Ok(None),
    }
}

fn int_number(number: &Number) -> Result<Option<i64>, ToolSignalError> {
    let literal = number.to_string();
    if !literal.contains(['.', 'e', 'E']) {
        return literal
            .parse::<i64>()
            .map(Some)
            .map_err(|_error| ToolSignalError::CounterOutOfRange);
    }
    let value: f64 = literal
        .parse()
        .map_err(|_error| ToolSignalError::CounterOutOfRange)?;
    finite_int(value)
}

fn int_text(text: &str) -> Result<Option<i64>, ToolSignalError> {
    let trimmed = text.trim();
    let (negative, digits) = trimmed.strip_prefix('-').map_or_else(
        || (false, trimmed.strip_prefix('+').unwrap_or(trimmed)),
        |rest| (true, rest),
    );
    if !valid_underscored_digits(digits) {
        return Ok(None);
    }
    let magnitude = digit_magnitude(digits).ok_or(ToolSignalError::CounterOutOfRange)?;
    let signed = if negative {
        magnitude
            .checked_neg()
            .ok_or(ToolSignalError::CounterOutOfRange)?
    } else {
        magnitude
    };
    i64::try_from(signed)
        .map(Some)
        .map_err(|_error| ToolSignalError::CounterOutOfRange)
}

fn finite_int(value: f64) -> Result<Option<i64>, ToolSignalError> {
    let decimal = Decimal::try_from(value).map_err(|_error| ToolSignalError::CounterOutOfRange)?;
    decimal
        .trunc()
        .to_i64()
        .map(Some)
        .ok_or(ToolSignalError::CounterOutOfRange)
}

fn quantity_from_json(
    value: &Json,
    field: QuantityField,
) -> Result<Option<Decimal>, ToolSignalError> {
    match value {
        Json::Null | Json::List(_) | Json::Object(_) => Ok(None),
        Json::Bool(flag) => Ok(Some(Decimal::from(u8::from(*flag)))),
        Json::Number(number) => number_quantity(number, field).map(Some),
        Json::Text(text) => text_quantity(text, field),
    }
}

fn number_quantity(number: &Number, field: QuantityField) -> Result<Decimal, ToolSignalError> {
    let value: f64 = number
        .to_string()
        .parse()
        .map_err(|_error| ToolSignalError::QuantityOutOfRange { field })?;
    finite_decimal(value, field)
}

fn text_quantity(text: &str, field: QuantityField) -> Result<Option<Decimal>, ToolSignalError> {
    let trimmed = text.trim();
    if !is_float_literal(trimmed) {
        return Ok(None);
    }
    let cleaned = trimmed.replace('_', "");
    let value: f64 = cleaned
        .parse()
        .map_err(|_error| ToolSignalError::QuantityOutOfRange { field })?;
    finite_decimal(value, field).map(Some)
}

fn finite_decimal(value: f64, field: QuantityField) -> Result<Decimal, ToolSignalError> {
    if !value.is_finite() {
        return Err(ToolSignalError::NonFiniteQuantity { field });
    }
    Decimal::try_from(value).map_err(|_error| ToolSignalError::QuantityOutOfRange { field })
}

mod tests;
