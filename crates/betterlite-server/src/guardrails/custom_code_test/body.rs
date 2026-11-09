use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::Deserialize;

use super::error::TestRouteError;

#[derive(Debug, Clone, PartialEq)]
pub struct TestCustomCodeRequest {
    pub custom_code: String,
    pub test_input: BTreeMap<String, GuardrailValue>,
    pub input_type: String,
    pub request_data: Option<BTreeMap<String, GuardrailValue>>,
}

#[derive(Debug, Deserialize)]
struct RawTestRequest {
    custom_code: String,
    test_input: BTreeMap<String, WireValue>,
    #[serde(default = "default_input_type")]
    input_type: String,
    #[serde(default)]
    request_data: Option<BTreeMap<String, WireValue>>,
}

// serde_json::Number decodes the private map token arbitrary_precision uses, which an i64/f64 untagged variant cannot
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum WireValue {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    Text(String),
    List(Vec<Self>),
    Object(BTreeMap<String, Self>),
}

fn default_input_type() -> String {
    "request".to_owned()
}

pub fn decode_test_request(body: &[u8]) -> Result<TestCustomCodeRequest, TestRouteError> {
    let mut deserializer = serde_json::Deserializer::from_slice(body);
    let raw: RawTestRequest = serde_path_to_error::deserialize(&mut deserializer)
        .map_err(|source| TestRouteError::Body { source })?;
    deserializer
        .end()
        .map_err(|source| TestRouteError::Trailing { source })?;
    Ok(TestCustomCodeRequest {
        custom_code: raw.custom_code,
        test_input: to_guardrail_map(raw.test_input)?,
        input_type: raw.input_type,
        request_data: raw.request_data.map(to_guardrail_map).transpose()?,
    })
}

fn to_guardrail_map(
    fields: BTreeMap<String, WireValue>,
) -> Result<BTreeMap<String, GuardrailValue>, TestRouteError> {
    let mut converted = BTreeMap::new();
    for (key, value) in fields {
        converted.insert(key, to_guardrail_value(value)?);
    }
    Ok(converted)
}

fn to_guardrail_value(value: WireValue) -> Result<GuardrailValue, TestRouteError> {
    let converted = match value {
        WireValue::Null => GuardrailValue::Null,
        WireValue::Bool(flag) => GuardrailValue::Boolean(flag),
        WireValue::Number(number) => number_to_guardrail(&number)?,
        WireValue::Text(text) => GuardrailValue::Text(text),
        WireValue::List(items) => {
            let mut converted = Vec::new();
            for item in items {
                converted.push(to_guardrail_value(item)?);
            }
            GuardrailValue::Array(converted)
        }
        WireValue::Object(fields) => GuardrailValue::Object(to_guardrail_map(fields)?),
    };
    Ok(converted)
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
