use std::collections::BTreeMap;
use std::str;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;
use serde_json::error::Category;

use super::failure::{BATCH_MISMATCH, Failure, UNEXPECTED_SHAPE, UNREADABLE};
use super::{CompressionResult, SAFE_RESPONSE_TEXT_LIMIT};

mod tests;

pub const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const UNDECODABLE_RESPONSE_BODY: &str = "<undecodable response body>";

// LiteLLM's _safe_response_text: reporting a failure must not raise while reading the body it reports
#[must_use]
pub fn safe_response_text(text: Option<&str>, limit: usize) -> String {
    text.map_or_else(
        || UNDECODABLE_RESPONSE_BODY.to_owned(),
        |body| body.chars().take(limit).collect(),
    )
}

// LiteLLM's _safe_int: a malformed token stat on a 200 response must not raise past the fail policy
#[must_use]
pub fn safe_int(value: Option<&GuardrailValue>) -> i64 {
    match value {
        Some(GuardrailValue::SignedInteger(number)) => *number,
        Some(GuardrailValue::UnsignedInteger(number)) => i64::try_from(*number).unwrap_or(i64::MAX),
        Some(GuardrailValue::Decimal(number)) => Decimal::from_f64(*number)
            .and_then(|decimal| i64::try_from(decimal.trunc()).ok())
            .unwrap_or(0),
        Some(GuardrailValue::Boolean(boolean)) => i64::from(*boolean),
        Some(GuardrailValue::Text(text)) => text.trim().parse::<i64>().unwrap_or(0),
        None
        | Some(GuardrailValue::Null | GuardrailValue::Array(_) | GuardrailValue::Object(_)) => 0,
    }
}

pub(super) fn parse_results(
    body: Option<&str>,
    expected: usize,
) -> Result<Vec<CompressionResult>, Failure> {
    let Some(text) = body else {
        return Err(Failure {
            message: UNREADABLE,
            detail: safe_response_text(None, SAFE_RESPONSE_TEXT_LIMIT),
        });
    };
    // OpaqueParams reads serde_json's arbitrary_precision number marker into the matching value,
    // so a float token stat stays a number instead of the private-key object an untagged enum sees
    let fields = match serde_json::from_str::<OpaqueParams>(text) {
        Ok(params) => params.0,
        Err(error) if error.classify() == Category::Data => {
            return Err(Failure {
                message: UNEXPECTED_SHAPE,
                detail: safe_response_text(Some(text), SAFE_RESPONSE_TEXT_LIMIT),
            });
        }
        Err(_unreadable) => {
            return Err(Failure {
                message: UNREADABLE,
                detail: safe_response_text(Some(text), SAFE_RESPONSE_TEXT_LIMIT),
            });
        }
    };
    let Some(GuardrailValue::Object(data)) = fields.get("data") else {
        return Err(Failure {
            message: UNEXPECTED_SHAPE,
            detail: safe_response_text(Some(text), SAFE_RESPONSE_TEXT_LIMIT),
        });
    };
    if expected == 1 {
        return Ok(vec![CompressionResult::from_fields(data)]);
    }
    batch_results(data, expected)
}

// Fuzz and corpus entry: arbitrary bytes go through the response parse without panicking
#[must_use]
pub fn parse_body(body: &[u8], expected: usize) -> Option<Vec<CompressionResult>> {
    let Ok(text) = str::from_utf8(body) else {
        return None;
    };
    parse_results(Some(text), expected).ok()
}

fn batch_results(
    data: &BTreeMap<String, GuardrailValue>,
    expected: usize,
) -> Result<Vec<CompressionResult>, Failure> {
    let Some(GuardrailValue::Array(results)) = data.get("results") else {
        return Err(Failure {
            message: BATCH_MISMATCH,
            detail: format!("expected {expected}, got none"),
        });
    };
    let mut parsed_results = Vec::with_capacity(results.len());
    for result in results {
        let GuardrailValue::Object(result_fields) = result else {
            return Err(Failure {
                message: BATCH_MISMATCH,
                detail: format!("expected {expected}, got {}", results.len()),
            });
        };
        parsed_results.push(CompressionResult::from_fields(result_fields));
    }
    if parsed_results.len() != expected {
        return Err(Failure {
            message: BATCH_MISMATCH,
            detail: format!("expected {expected}, got {}", parsed_results.len()),
        });
    }
    Ok(parsed_results)
}
