use std::fmt::{self, Write as _};
use std::num::{ParseFloatError, ParseIntError};
use std::ops::Range;

use indexmap::IndexMap;
use serde::Serialize;
use serde_json::value::RawValue;
use sha2::{Digest as _, Sha256};

const FIXED_EXPONENTS: Range<i32> = -4_i32..16_i32;

#[derive(Debug, thiserror::Error)]
pub(super) enum JsonError {
    #[error("the JSON value cannot be read")]
    Json(#[from] serde_json::Error),
    #[error("the compact JSON text cannot be written")]
    Format(#[from] fmt::Error),
    #[error("the float cannot be read")]
    Float(#[from] ParseFloatError),
    #[error("the float exponent cannot be read")]
    Exponent(#[from] ParseIntError),
    #[error("the number {0} has no finite float value")]
    NotFinite(String),
    #[error("the float {0} printed without an exponent")]
    Scientific(String),
}

// LiteLLM hashes json.dumps of the parsed body, so this writes what Python's json module would
pub(super) fn compact(raw: &RawValue) -> Result<Box<RawValue>, JsonError> {
    let mut text = String::new();
    write_compact(raw, &mut text)?;
    Ok(RawValue::from_string(text)?)
}

fn write_compact(raw: &RawValue, out: &mut String) -> Result<(), JsonError> {
    let source = raw.get();
    let mut separator = "";
    match source.as_bytes().first() {
        Some(b'{') => {
            let object: IndexMap<String, &RawValue> = serde_json::from_str(source)?;
            out.push('{');
            for (key, value) in object {
                write!(out, "{separator}{}:", serde_json::to_string(&key)?)?;
                write_compact(value, out)?;
                separator = ",";
            }
            out.push('}');
        }
        Some(b'[') => {
            let items: Vec<&RawValue> = serde_json::from_str(source)?;
            out.push('[');
            for item in items {
                out.push_str(separator);
                write_compact(item, out)?;
                separator = ",";
            }
            out.push(']');
        }
        Some(b'"') => {
            let text: String = serde_json::from_str(source)?;
            out.push_str(&serde_json::to_string(&text)?);
        }
        Some(b'-' | b'0'..=b'9') => write_number(source, out)?,
        Some(_) | None => out.push_str(source),
    }
    Ok(())
}

// Python keeps JSON integers exact and prints floats with repr: 1e-05, 1.5e+16, 100.0
fn write_number(source: &str, out: &mut String) -> Result<(), JsonError> {
    if !source.contains(['.', 'e', 'E']) {
        out.push_str(if source == "-0" { "0" } else { source });
        return Ok(());
    }
    let value: f64 = source.parse()?;
    if !value.is_finite() {
        return Err(JsonError::NotFinite(source.to_owned()));
    }
    let scientific = format!("{value:e}");
    let (mantissa, power) = scientific
        .split_once('e')
        .ok_or_else(|| JsonError::Scientific(scientific.clone()))?;
    let exponent: i32 = power.parse()?;
    if FIXED_EXPONENTS.contains(&exponent) {
        let fixed = value.to_string();
        let point = if fixed.contains('.') { "" } else { ".0" };
        write!(out, "{fixed}{point}")?;
    } else {
        let sign = if exponent < 0_i32 { '-' } else { '+' };
        write!(out, "{mantissa}e{sign}{:02}", exponent.unsigned_abs())?;
    }
    Ok(())
}

pub(super) fn digest<Value>(value: &Value) -> Result<String, serde_json::Error>
where
    Value: Serialize,
{
    let mut hasher = Sha256::new();
    serde_json::to_writer(&mut hasher, value)?;
    Ok(format!("{:x}", hasher.finalize()))
}
