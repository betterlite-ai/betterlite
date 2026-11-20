use std::str;

use betterlite_config::sections::settings::JsonValue;

use super::error::{ComplianceError, Issue, LocPart};
use super::model::{ComplianceRequest, GuardrailEvidence, GuardrailMode, GuardrailStatus};

const REQUIRED: &str = "Field required";
const STRING: &str = "Input should be a valid string";
const LIST: &str = "Input should be a valid list";
const DICTIONARY: &str = "Input should be a valid dictionary";
const OBJECT: &str = "Input should be a valid dictionary or object to extract fields from";
const JSON_DECODE: &str = "JSON decode error";
const BODY: &str = "body";
const REQUEST_ID: &str = "request_id";
const USER_ID: &str = "user_id";
const MODEL: &str = "model";
const TIMESTAMP: &str = "timestamp";
const GUARDRAIL_INFORMATION: &str = "guardrail_information";
const GUARDRAIL_STATUS: &str = "guardrail_status";
const GUARDRAIL_MODE: &str = "guardrail_mode";

#[derive(Clone, Copy)]
enum Optionality {
    Required,
    Optional,
}

pub fn decode_compliance_request(bytes: &[u8]) -> Result<ComplianceRequest, ComplianceError> {
    if bytes.is_empty() {
        return Err(ComplianceError::Request(vec![body_issue(
            "missing", REQUIRED,
        )]));
    }
    let parsed = str::from_utf8(bytes)
        .ok()
        .and_then(|text| serde_json::from_str::<JsonValue>(text).ok());
    let Some(value) = parsed else {
        return Err(ComplianceError::Request(vec![body_issue(
            "json_invalid",
            JSON_DECODE,
        )]));
    };
    checked(&value)
}

fn checked(value: &JsonValue) -> Result<ComplianceRequest, ComplianceError> {
    if !matches!(value, JsonValue::Object(_)) {
        return Err(ComplianceError::Request(vec![body_issue(
            "model_attributes_type",
            OBJECT,
        )]));
    }
    let mut issues = Vec::new();
    let request_id = string_field(value, REQUEST_ID, Optionality::Required, &mut issues);
    let user_id = string_field(value, USER_ID, Optionality::Optional, &mut issues);
    let model = string_field(value, MODEL, Optionality::Optional, &mut issues);
    let timestamp = string_field(value, TIMESTAMP, Optionality::Optional, &mut issues);
    let guardrail_information = guardrails(value, &mut issues);
    if !issues.is_empty() {
        return Err(ComplianceError::Request(issues));
    }
    let Some(found_id) = request_id else {
        return Err(ComplianceError::Request(vec![field_issue(
            "missing", REQUEST_ID, REQUIRED,
        )]));
    };
    Ok(ComplianceRequest {
        request_id: found_id,
        user_id,
        model,
        timestamp,
        guardrail_information,
    })
}

fn string_field(
    body: &JsonValue,
    name: &'static str,
    optionality: Optionality,
    issues: &mut Vec<Issue>,
) -> Option<String> {
    let required = matches!(optionality, Optionality::Required);
    match field(body, name) {
        None => {
            if required {
                issues.push(field_issue("missing", name, REQUIRED));
            }
            None
        }
        Some(JsonValue::Null) => {
            if required {
                issues.push(field_issue("string_type", name, STRING));
            }
            None
        }
        Some(JsonValue::String(text)) => Some(text.clone()),
        Some(
            JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Array(_) | JsonValue::Object(_),
        ) => {
            issues.push(field_issue("string_type", name, STRING));
            None
        }
    }
}

fn guardrails(body: &JsonValue, issues: &mut Vec<Issue>) -> Vec<GuardrailEvidence> {
    match field(body, GUARDRAIL_INFORMATION) {
        None | Some(JsonValue::Null) => Vec::new(),
        Some(JsonValue::Array(items)) => items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| evidence(item, index, issues))
            .collect(),
        Some(
            JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::String(_) | JsonValue::Object(_),
        ) => {
            issues.push(field_issue("list_type", GUARDRAIL_INFORMATION, LIST));
            Vec::new()
        }
    }
}

fn evidence(item: &JsonValue, index: usize, issues: &mut Vec<Issue>) -> Option<GuardrailEvidence> {
    if !matches!(item, JsonValue::Object(_)) {
        issues.push(Issue {
            kind: "dict_type",
            loc: vec![
                LocPart::Name(BODY),
                LocPart::Name(GUARDRAIL_INFORMATION),
                LocPart::Index(index),
            ],
            msg: DICTIONARY.to_owned(),
        });
        return None;
    }
    Some(GuardrailEvidence {
        status: GuardrailStatus::from_value(field(item, GUARDRAIL_STATUS)),
        mode: GuardrailMode::from_value(field(item, GUARDRAIL_MODE)),
    })
}

fn field<'value>(body: &'value JsonValue, name: &str) -> Option<&'value JsonValue> {
    let JsonValue::Object(fields) = body else {
        return None;
    };
    fields.get(name)
}

fn body_issue(kind: &'static str, message: &'static str) -> Issue {
    Issue {
        kind,
        loc: vec![LocPart::Name(BODY)],
        msg: message.to_owned(),
    }
}

fn field_issue(kind: &'static str, name: &'static str, message: &'static str) -> Issue {
    Issue {
        kind,
        loc: vec![LocPart::Name(BODY), LocPart::Name(name)],
        msg: message.to_owned(),
    }
}
