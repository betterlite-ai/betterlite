use betterlite_config::sections::settings::JsonValue;

use super::error::{EstimateError, Issue};
use crate::proxy_request_contracts::ContractError;
use crate::proxy_request_contracts::cost_estimate::CostEstimateRequest;

const REQUIRED: &str = "Field required";
const INTEGER: &str = "Input should be a valid integer";
const STRING: &str = "Input should be a valid string";
const DICTIONARY: &str = "Input should be a valid dictionary or object to extract fields from";
const GREATER_THAN_EQUAL: &str = "Input should be greater than or equal to 0";
const JSON_DECODE: &str = "JSON decode error";

pub fn decode_cost_estimate_request(bytes: &[u8]) -> Result<CostEstimateRequest, EstimateError> {
    if bytes.is_empty() {
        return Err(EstimateError::Request(vec![Issue {
            kind: "missing",
            loc: vec!["body".to_owned()],
            msg: REQUIRED.to_owned(),
        }]));
    }
    let value = std::str::from_utf8(bytes)
        .ok()
        .and_then(|text| serde_json::from_str::<JsonValue>(text).ok());
    let Some(value) = value else {
        return Err(EstimateError::Request(vec![Issue {
            kind: "json_invalid",
            loc: vec!["body".to_owned()],
            msg: JSON_DECODE.to_owned(),
        }]));
    };
    checked(&value)
}

fn checked(value: &JsonValue) -> Result<CostEstimateRequest, EstimateError> {
    if !matches!(value, JsonValue::Object(_)) {
        return Err(EstimateError::Request(vec![Issue {
            kind: "model_attributes_type",
            loc: vec!["body".to_owned()],
            msg: DICTIONARY.to_owned(),
        }]));
    }
    let mut issues = Vec::new();
    check_model(value, &mut issues);
    check_required_count(value, "input_tokens", &mut issues);
    check_required_count(value, "output_tokens", &mut issues);
    for name in [
        "cache_read_input_tokens",
        "cache_creation_input_tokens",
        "reasoning_tokens",
    ] {
        check_default_count(value, name, &mut issues);
    }
    for name in ["num_requests_per_day", "num_requests_per_month"] {
        check_optional_count(value, name, &mut issues);
    }
    if !issues.is_empty() {
        return Err(EstimateError::Request(issues));
    }
    CostEstimateRequest::from_value(value).map_err(shared_error)
}

fn shared_error(error: ContractError) -> EstimateError {
    let message = match error {
        ContractError::Validation { message } => message,
        ContractError::Decode { source } => source.to_string(),
    };
    EstimateError::Request(vec![Issue {
        kind: "value_error",
        loc: vec!["body".to_owned()],
        msg: format!("Value error, {message}"),
    }])
}

fn field<'value>(body: &'value JsonValue, name: &str) -> Option<&'value JsonValue> {
    let JsonValue::Object(fields) = body else {
        return None;
    };
    fields.get(name)
}

fn field_issue(kind: &'static str, name: &str, message: &'static str) -> Issue {
    Issue {
        kind,
        loc: vec!["body".to_owned(), name.to_owned()],
        msg: message.to_owned(),
    }
}

fn check_model(body: &JsonValue, issues: &mut Vec<Issue>) {
    match field(body, "model") {
        None => issues.push(field_issue("missing", "model", REQUIRED)),
        Some(JsonValue::String(_)) => {}
        Some(_) => issues.push(field_issue("string_type", "model", STRING)),
    }
}

fn check_required_count(body: &JsonValue, name: &str, issues: &mut Vec<Issue>) {
    match field(body, name) {
        None => issues.push(field_issue("missing", name, REQUIRED)),
        Some(value) => count_value(value, name, issues),
    }
}

fn check_default_count(body: &JsonValue, name: &str, issues: &mut Vec<Issue>) {
    if let Some(value) = field(body, name) {
        count_value(value, name, issues);
    }
}

fn check_optional_count(body: &JsonValue, name: &str, issues: &mut Vec<Issue>) {
    match field(body, name) {
        None | Some(JsonValue::Null) => {}
        Some(value) => count_value(value, name, issues),
    }
}

// An integer field takes a JSON integer only: pydantic's lax mode would also read strings and bools.
fn count_value(value: &JsonValue, name: &str, issues: &mut Vec<Issue>) {
    let Some(count) = value.as_i64() else {
        issues.push(field_issue("int_type", name, INTEGER));
        return;
    };
    if count < 0 {
        issues.push(field_issue("greater_than_equal", name, GREATER_THAN_EQUAL));
    }
}
