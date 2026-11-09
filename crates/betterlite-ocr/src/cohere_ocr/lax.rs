use serde::Deserialize;
use serde_json::Number;
use thiserror::Error;

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum LaxIntegerWire {
    Number(Number),
    Text(String),
    Bool(bool),
}

#[derive(Debug, Error)]
#[error("expected an integer in the i64 range")]
struct LaxIntegerError;

#[derive(Debug, Deserialize)]
#[serde(try_from = "LaxIntegerWire")]
pub(crate) struct LaxInteger(pub(crate) i64);

impl TryFrom<LaxIntegerWire> for LaxInteger {
    type Error = LaxIntegerError;

    fn try_from(wire: LaxIntegerWire) -> Result<Self, Self::Error> {
        let value = match wire {
            LaxIntegerWire::Number(value) => integer_from_number(&value).ok_or(LaxIntegerError)?,
            LaxIntegerWire::Text(value) => parse_integer(value.trim()).ok_or(LaxIntegerError)?,
            LaxIntegerWire::Bool(value) => i64::from(value),
        };
        Ok(Self(value))
    }
}

fn integer_from_number(number: &Number) -> Option<i64> {
    if let Some(value) = number.as_i64() {
        return Some(value);
    }
    if let Some(value) = number.as_u64() {
        return i64::try_from(value).ok();
    }
    number.as_f64().and_then(integral)
}

fn integral(value: f64) -> Option<i64> {
    if !value.is_finite() || value.fract() != 0.0_f64 {
        return None;
    }
    // Rust has no checked float-to-integer conversion, and an as cast would truncate silently
    format!("{value:.0}").parse::<i64>().ok()
}

fn parse_integer(value: &str) -> Option<i64> {
    let integer = match value.split_once('.') {
        Some((integer, fraction))
            if !fraction.is_empty() && fraction.bytes().all(|digit| digit == b'0') =>
        {
            integer
        }
        Some(_) => return None,
        None => value,
    };
    if integer.starts_with('_') || integer.ends_with('_') || integer.contains("__") {
        return None;
    }
    let digits = integer.strip_prefix(['+', '-']).unwrap_or(integer);
    if digits.starts_with('_') {
        return None;
    }
    integer.replace('_', "").parse::<i64>().ok()
}
