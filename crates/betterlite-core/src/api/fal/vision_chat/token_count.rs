use std::str::FromStr;

use betterlite_models::decisions::Json;
use num_traits::ToPrimitive;
use rust_decimal::Decimal;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer};

// a token count read the way pydantic reads an int: whole numbers, integral decimals, numeric strings and booleans
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenCount(pub u64);

impl<'de> Deserialize<'de> for TokenCount {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let count = match Json::deserialize(deserializer)? {
            Json::Bool(flag) => Some(u64::from(flag)),
            Json::Number(number) => number_count(&number.to_string()),
            Json::Text(text) => text_count(&text),
            Json::Null | Json::List(_) | Json::Object(_) => None,
        };
        count.map(Self).ok_or_else(|| {
            D::Error::custom("a token count must be a whole number from 0 to 2^64-1")
        })
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

// pydantic trims the string, takes one sign, single underscores between digits and a fraction of zeros only
fn text_count(text: &str) -> Option<u64> {
    let trimmed = text.trim();
    let unsigned = trimmed.strip_prefix(['+', '-']).unwrap_or(trimmed);
    let (integral, fraction) = unsigned.split_once('.').unzip();
    let whole = integral.unwrap_or(unsigned);
    if let Some(zeros) = fraction
        && (zeros.is_empty() || !zeros.bytes().all(|digit| digit == b'0'))
    {
        return None;
    }
    let groups: Vec<&str> = whole.split('_').collect();
    if groups
        .iter()
        .any(|group| group.is_empty() || !group.bytes().all(|digit| digit.is_ascii_digit()))
    {
        return None;
    }
    let count: u64 = groups.concat().parse().ok()?;
    if trimmed.starts_with('-') && count != 0 {
        return None;
    }
    Some(count)
}
