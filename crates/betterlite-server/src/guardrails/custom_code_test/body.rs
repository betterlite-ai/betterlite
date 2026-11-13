use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::Deserialize;
use serde_json::value::RawValue;

use super::error::TestRouteError;

const MAX_VALUE_DEPTH: usize = 64;

#[derive(Debug, Clone, PartialEq)]
pub struct TestCustomCodeRequest {
    pub custom_code: String,
    pub test_input: BTreeMap<String, GuardrailValue>,
    pub input_type: String,
    pub request_data: Option<BTreeMap<String, GuardrailValue>>,
}

#[derive(Debug, Deserialize)]
struct RawTestRequest<'body> {
    custom_code: String,
    #[serde(borrow)]
    test_input: BTreeMap<String, &'body RawValue>,
    #[serde(default = "default_input_type")]
    input_type: String,
    #[serde(default, borrow)]
    request_data: Option<BTreeMap<String, &'body RawValue>>,
}

fn default_input_type() -> String {
    "request".to_owned()
}

pub fn decode_test_request(body: &[u8]) -> Result<TestCustomCodeRequest, TestRouteError> {
    let mut deserializer = serde_json::Deserializer::from_slice(body);
    let raw: RawTestRequest<'_> = serde_path_to_error::deserialize(&mut deserializer)
        .map_err(|source| TestRouteError::Body { source })?;
    deserializer
        .end()
        .map_err(|source| TestRouteError::Trailing { source })?;
    Ok(TestCustomCodeRequest {
        custom_code: raw.custom_code,
        test_input: to_guardrail_map(raw.test_input, 1)?,
        input_type: raw.input_type,
        request_data: raw
            .request_data
            .map(|fields| to_guardrail_map(fields, 1))
            .transpose()?,
    })
}

// RawValue captures each child as a slice of the body, so a level is read without buffering the levels below it
fn to_guardrail_map(
    fields: BTreeMap<String, &RawValue>,
    depth: usize,
) -> Result<BTreeMap<String, GuardrailValue>, TestRouteError> {
    let mut converted = BTreeMap::new();
    for (key, raw) in fields {
        converted.insert(key, to_guardrail_value(raw, depth)?);
    }
    Ok(converted)
}

fn to_guardrail_value(raw: &RawValue, depth: usize) -> Result<GuardrailValue, TestRouteError> {
    if depth > MAX_VALUE_DEPTH {
        return Err(TestRouteError::Depth {
            limit: MAX_VALUE_DEPTH,
        });
    }
    let text = raw.get().trim_start();
    match text.as_bytes().first() {
        Some(b'{') => {
            let fields: BTreeMap<String, &RawValue> =
                serde_json::from_str(text).map_err(|source| TestRouteError::Value { source })?;
            Ok(GuardrailValue::Object(to_guardrail_map(
                fields,
                depth.saturating_add(1),
            )?))
        }
        Some(b'[') => {
            let items: Vec<&RawValue> =
                serde_json::from_str(text).map_err(|source| TestRouteError::Value { source })?;
            let mut converted = Vec::new();
            for item in items {
                converted.push(to_guardrail_value(item, depth.saturating_add(1))?);
            }
            Ok(GuardrailValue::Array(converted))
        }
        Some(b'"') => {
            let text_value: String =
                serde_json::from_str(text).map_err(|source| TestRouteError::Value { source })?;
            Ok(GuardrailValue::Text(text_value))
        }
        Some(b't') => Ok(GuardrailValue::Boolean(true)),
        Some(b'f') => Ok(GuardrailValue::Boolean(false)),
        Some(b'n') => Ok(GuardrailValue::Null),
        _ => {
            let number: serde_json::Number =
                serde_json::from_str(text).map_err(|source| TestRouteError::Value { source })?;
            number_to_guardrail(&number)
        }
    }
}

fn number_to_guardrail(number: &serde_json::Number) -> Result<GuardrailValue, TestRouteError> {
    if let Some(value) = number.as_i64() {
        return Ok(GuardrailValue::SignedInteger(value));
    }
    if let Some(value) = number.as_u64() {
        return Ok(GuardrailValue::UnsignedInteger(value));
    }
    number
        .as_f64()
        .map(GuardrailValue::Decimal)
        .ok_or_else(|| TestRouteError::Number {
            value: number.to_string(),
        })
}
