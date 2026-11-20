use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::Json as PromptJson;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::error::CreateError;

// The two Json enums and the provider contract's maps convert through their serde forms
pub(crate) fn round_trip<Source, Target>(value: &Source) -> Result<Target, CreateError>
where
    Source: Serialize + ?Sized,
    Target: DeserializeOwned,
{
    serde_json::from_str(&serde_json::to_string(value).map_err(CreateError::Encode)?)
        .map_err(CreateError::Encode)
}

// Python's type names, as float() and the type errors print them
pub(crate) trait ValueKind {
    fn value_kind(&self) -> &'static str;
}

// The two Json enums carry the same variants, so one body implements both
macro_rules! value_kinds {
    ($($json:ty),+ $(,)?) => {$(
        impl ValueKind for $json {
            fn value_kind(&self) -> &'static str {
                match self {
                    Self::Null => "NoneType",
                    Self::Bool(_) => "bool",
                    Self::Text(_) => "str",
                    Self::List(_) => "list",
                    Self::Object(_) => "dict",
                    Self::Number(number) => number_kind(&number.to_string()),
                }
            }
        }
    )+};
}

value_kinds!(Json, PromptJson);

fn number_kind(text: &str) -> &'static str {
    if text.contains(|character: char| matches!(character, '.' | 'e' | 'E')) {
        "float"
    } else {
        "int"
    }
}

// float(value) for the shapes an optional timer can carry; Display is Python's own message
#[derive(Debug, thiserror::Error)]
pub(crate) enum FloatError {
    #[error("float() argument must be a string or a real number, not '{kind}'")]
    NotANumber { kind: &'static str },
    #[error("could not convert string to float: '{text}'")]
    Text { text: String },
}

pub(crate) fn float_value(value: &PromptJson) -> Result<f64, FloatError> {
    match value {
        PromptJson::Bool(flag) => Ok(if *flag { 1.0_f64 } else { 0.0_f64 }),
        PromptJson::Number(number) => Ok(number.as_f64().unwrap_or(f64::INFINITY)),
        PromptJson::Text(text) => {
            parse_float_text(text).map_err(|()| FloatError::Text { text: text.clone() })
        }
        other => Err(FloatError::NotANumber {
            kind: other.value_kind(),
        }),
    }
}

fn parse_float_text(text: &str) -> Result<f64, ()> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(());
    }
    let (negative, rest) = match trimmed.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, trimmed.strip_prefix('+').unwrap_or(trimmed)),
    };
    let lowered = rest.to_ascii_lowercase();
    if matches!(lowered.as_str(), "inf" | "infinity") {
        return Ok(if negative {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        });
    }
    if lowered == "nan" {
        return Ok(f64::NAN);
    }
    if !valid_decimal(rest) {
        return Err(());
    }
    trimmed.replace('_', "").parse::<f64>().map_err(|_| ())
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
        && bytes
            .get(index.saturating_add(1))
            .is_some_and(u8::is_ascii_digit)
        && bytes
            .get(index.saturating_sub(1))
            .is_some_and(u8::is_ascii_digit)
}
