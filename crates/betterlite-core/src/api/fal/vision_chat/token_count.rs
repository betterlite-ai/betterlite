use std::str::FromStr;

use betterlite_models::decisions::Json;
use num_traits::ToPrimitive;
use rust_decimal::Decimal;
use serde::Deserialize;
use thiserror::Error;

// a token count read the way pydantic reads an int: whole numbers, integral decimals, numeric strings and booleans
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(try_from = "Json")]
pub struct TokenCount(pub u64);

#[derive(Debug, Error)]
pub enum TokenCountError {
    #[error("a token count must be a whole number from 0 to 2^64-1")]
    NotAWholeNumber,
}

impl TryFrom<Json> for TokenCount {
    type Error = TokenCountError;

    fn try_from(value: Json) -> Result<Self, Self::Error> {
        let count = match value {
            Json::Bool(flag) => Some(u64::from(flag)),
            Json::Number(number) => number_count(&number.to_string()),
            Json::Text(text) => text_count(&text),
            Json::Null | Json::List(_) | Json::Object(_) => None,
        };
        count.map(Self).ok_or(TokenCountError::NotAWholeNumber)
    }
}

// the JSON token may carry an exponent, as in 1e2, and an integral fraction, as in 3.0
fn number_count(token: &str) -> Option<u64> {
    let decimal = if token.contains(['e', 'E']) {
        Decimal::from_scientific(token)
    } else {
        Decimal::from_str(token)
    }
    .ok()?;
    if decimal.is_zero() {
        return Some(0);
    }
    if decimal.fract().is_zero() {
        decimal.to_u64()
    } else {
        None
    }
}

// pydantic trims the string, takes one sign, strips a leading run of zeros and underscores, then reads a JSON integer
fn text_count(text: &str) -> Option<u64> {
    let trimmed = text.trim();
    let unsigned = trimmed.strip_prefix(['+', '-']).unwrap_or(trimmed);
    let magnitude = count_digits(strip_leading_zeros(unsigned)?)?;
    let signed = if trimmed.starts_with('-') {
        format!("-{magnitude}")
    } else {
        magnitude
    };
    json_integer(&signed)
}

// pydantic keeps the last character of an all-zero run, so 00 reads as 0 while 0_ keeps its underscore and fails
fn strip_leading_zeros(text: &str) -> Option<&str> {
    let mut characters = text.char_indices();
    let (first_index, first) = characters.next()?;
    if first != '0' {
        return first.is_ascii_digit().then_some(text);
    }
    let mut previous = first_index;
    for (index, character) in characters {
        match character {
            '0' | '_' => previous = index,
            '1'..='9' | '-' => return text.get(index..),
            '.' => return text.get(previous..),
            _ => return None,
        }
    }
    text.get(previous..)
}

// pydantic drops a fraction of zeros and the underscores that sit between two digits
fn count_digits(whole: &str) -> Option<String> {
    let without_fraction = match whole.split_once('.') {
        Some((integer, fraction))
            if !fraction.is_empty() && fraction.bytes().all(|digit| digit == b'0') =>
        {
            integer
        }
        _ => whole,
    };
    if !without_fraction.contains('_') {
        return Some(without_fraction.to_owned());
    }
    if without_fraction.starts_with('_')
        || without_fraction.ends_with('_')
        || without_fraction.contains("__")
    {
        return None;
    }
    Some(without_fraction.replace('_', ""))
}

// the cleaned text still has to read as a JSON integer, and a count below zero is only kept for -0
fn json_integer(text: &str) -> Option<u64> {
    let magnitude = text.strip_prefix('-').unwrap_or(text);
    if text.starts_with('-') && magnitude.bytes().any(|digit| digit != b'0') {
        return None;
    }
    serde_json::from_str(magnitude).ok()
}
