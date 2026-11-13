use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

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
        Some(GuardrailValue::Decimal(number)) => i64::try_from(*number).unwrap_or(0),
        Some(GuardrailValue::Text(text)) => text.trim().parse::<i64>().unwrap_or(0),
        None
        | Some(
            GuardrailValue::Null
            | GuardrailValue::Boolean(_)
            | GuardrailValue::Array(_)
            | GuardrailValue::Object(_),
        ) => 0,
    }
}

pub(super) async fn read_body(mut response: reqwest::Response) -> Option<String> {
    let mut body = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                    return None;
                }
                body.extend_from_slice(&chunk);
            }
            Ok(None) => break,
            Err(_error) => return None,
        }
    }
    String::from_utf8(body).ok()
}

pub(super) fn parse_results(
    body: Option<&str>,
    expected: usize,
) -> Result<Vec<CompressionResult>, Failure> {
    let Some(body) = body else {
        return Err(Failure {
            message: UNREADABLE,
            detail: safe_response_text(None, SAFE_RESPONSE_TEXT_LIMIT),
        });
    };
    let Ok(parsed) = serde_json::from_str::<GuardrailValue>(body) else {
        return Err(Failure {
            message: UNREADABLE,
            detail: safe_response_text(Some(body), SAFE_RESPONSE_TEXT_LIMIT),
        });
    };
    let GuardrailValue::Object(fields) = parsed else {
        return Err(Failure {
            message: UNEXPECTED_SHAPE,
            detail: safe_response_text(Some(body), SAFE_RESPONSE_TEXT_LIMIT),
        });
    };
    let Some(GuardrailValue::Object(data)) = fields.get("data") else {
        return Err(Failure {
            message: UNEXPECTED_SHAPE,
            detail: safe_response_text(Some(body), SAFE_RESPONSE_TEXT_LIMIT),
        });
    };
    if expected == 1 {
        return Ok(vec![CompressionResult::from_fields(data)]);
    }
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
