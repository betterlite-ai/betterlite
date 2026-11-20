use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::Json as PromptJson;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::value::{RawValue, to_raw_value};

use super::error::CreateError;

// The two Json enums and the provider contract's maps convert through their serde forms
pub(crate) fn round_trip<Source, Target>(value: &Source) -> Result<Target, CreateError>
where
    Source: Serialize + ?Sized,
    Target: DeserializeOwned,
{
    let text = serde_json::to_string(value).map_err(CreateError::Encode)?;
    serde_json::from_str(&text).map_err(CreateError::Encode)
}

pub(crate) fn raw_value<Source>(value: &Source) -> Result<Box<RawValue>, CreateError>
where
    Source: Serialize + ?Sized,
{
    to_raw_value(value).map_err(CreateError::Encode)
}

// fake_stream is checked with `is True`, so only the boolean true satisfies it
pub(crate) fn is_true(value: Option<&Json>) -> bool {
    matches!(value, Some(Json::Bool(true)))
}

// Python's type names, as float() and the type errors print them
pub(crate) trait PythonValue {
    fn python_type(&self) -> &'static str;
}

impl PythonValue for Json {
    fn python_type(&self) -> &'static str {
        match self {
            Self::Null => "NoneType",
            Self::Bool(_) => "bool",
            Self::Text(_) => "str",
            Self::List(_) => "list",
            Self::Object(_) => "dict",
            Self::Number(number) => number_type_name(&number.to_string()),
        }
    }
}

impl PythonValue for PromptJson {
    fn python_type(&self) -> &'static str {
        match self {
            Self::Null => "NoneType",
            Self::Bool(_) => "bool",
            Self::Text(_) => "str",
            Self::List(_) => "list",
            Self::Object(_) => "dict",
            Self::Number(number) => number_type_name(&number.to_string()),
        }
    }
}

fn number_type_name(text: &str) -> &'static str {
    if text.contains(|character: char| matches!(character, '.' | 'e' | 'E')) {
        "float"
    } else {
        "int"
    }
}

// float(value) for the shapes an optional timer can carry; the message is Python's own text
pub(crate) fn python_float(value: &PromptJson) -> Result<f64, String> {
    match value {
        PromptJson::Bool(flag) => Ok(if *flag { 1.0_f64 } else { 0.0_f64 }),
        PromptJson::Number(number) => Ok(number.as_f64().unwrap_or(f64::INFINITY)),
        PromptJson::Text(text) => parse_float_text(text).map_err(|()| float_text_error(text)),
        other => Err(format!(
            "float() argument must be a string or a real number, not '{}'",
            other.python_type()
        )),
    }
}

fn float_text_error(text: &str) -> String {
    format!("could not convert string to float: '{text}'")
}

fn parse_float_text(text: &str) -> Result<f64, ()> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(());
    }
    let (sign, rest) = match trimmed.strip_prefix('-') {
        Some(rest) => (-1.0_f64, rest),
        None => (1.0_f64, trimmed.strip_prefix('+').unwrap_or(trimmed)),
    };
    let lowered = rest.to_ascii_lowercase();
    if matches!(lowered.as_str(), "inf" | "infinity") {
        return Ok(sign * f64::INFINITY);
    }
    if lowered == "nan" {
        return Ok(f64::NAN);
    }
    if !valid_decimal(rest) {
        return Err(());
    }
    rest.replace('_', "")
        .parse::<f64>()
        .map(|value| sign * value)
        .map_err(|_| ())
}

// Python allows digits, one dot, underscores between digits and one exponent at the end
fn valid_decimal(text: &str) -> bool {
    let mut parts = text.splitn(2, |character: char| matches!(character, 'e' | 'E'));
    let mantissa = parts.next().unwrap_or("");
    if !valid_mantissa(mantissa) {
        return false;
    }
    match parts.next() {
        None => true,
        Some(exponent) => {
            let digits = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
            valid_digits(digits)
        }
    }
}

fn valid_mantissa(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut dots = 0_usize;
    let mut digits = 0_usize;
    for (index, byte) in bytes.iter().enumerate() {
        match byte {
            b'0'..=b'9' => digits = digits.saturating_add(1),
            b'.' => dots = dots.saturating_add(1),
            b'_' => {
                if !digit_neighbour(bytes, index) {
                    return false;
                }
            }
            _ => return false,
        }
    }
    digits > 0 && dots <= 1
}

fn valid_digits(text: &str) -> bool {
    let bytes = text.as_bytes();
    !bytes.is_empty()
        && bytes.iter().enumerate().all(|(index, byte)| match byte {
            b'0'..=b'9' => true,
            b'_' => digit_neighbour(bytes, index),
            _ => false,
        })
}

fn digit_neighbour(bytes: &[u8], index: usize) -> bool {
    index > 0
        && bytes.get(index + 1).is_some_and(u8::is_ascii_digit)
        && bytes.get(index - 1).is_some_and(u8::is_ascii_digit)
}
