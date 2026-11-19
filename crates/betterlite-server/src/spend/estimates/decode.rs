use std::str;

use betterlite_models::decisions::Json;

use super::error::{EstimateError, Issue};

const REQUIRED: &str = "Field required";
const STRING: &str = "Input should be a valid string";
const LIST: &str = "Input should be a valid list";
const DICTIONARY: &str = "Input should be a valid dictionary";
const OBJECT: &str = "Input should be a valid dictionary or object to extract fields from";
const JSON_DECODE: &str = "JSON decode error";

// SpendCalculateRequest's three optional fields, each already narrowed to the type pydantic declares.
pub struct SpendCalculateRequest {
    pub model: Option<String>,
    pub messages: Option<Vec<Json>>,
    pub completion_response: Option<Json>,
}

// An empty body and malformed JSON answer like FastAPI's request validation; every field is optional.
pub fn decode_spend_estimate(bytes: &[u8]) -> Result<SpendCalculateRequest, EstimateError> {
    if bytes.is_empty() {
        return Err(request(vec![body_issue("missing", REQUIRED)]));
    }
    let parsed = str::from_utf8(bytes)
        .ok()
        .and_then(|text| serde_json::from_str::<Json>(text).ok());
    let Some(parsed) = parsed else {
        return Err(request(vec![body_issue("json_invalid", JSON_DECODE)]));
    };
    let Json::Object(fields) = parsed else {
        return Err(request(vec![body_issue("model_attributes_type", OBJECT)]));
    };
    let mut issues = Vec::new();
    let model = string_field(fields.get("model"), "model", &mut issues);
    let messages = list_field(fields.get("messages"), "messages", &mut issues);
    let completion_response = dict_field(fields.get("completion_response"), &mut issues);
    if !issues.is_empty() {
        return Err(EstimateError::Request(issues));
    }
    Ok(SpendCalculateRequest {
        model,
        messages,
        completion_response,
    })
}

fn request(issues: Vec<Issue>) -> EstimateError {
    EstimateError::Request(issues)
}

fn body_issue(kind: &'static str, message: &'static str) -> Issue {
    Issue {
        kind,
        loc: vec!["body".to_owned()],
        msg: message.to_owned(),
    }
}

fn field_issue(kind: &'static str, name: &str, message: &'static str) -> Issue {
    Issue {
        kind,
        loc: vec!["body".to_owned(), name.to_owned()],
        msg: message.to_owned(),
    }
}

fn string_field(value: Option<&Json>, name: &str, issues: &mut Vec<Issue>) -> Option<String> {
    match value {
        None | Some(Json::Null) => None,
        Some(Json::Text(text)) => Some(text.clone()),
        Some(_) => {
            issues.push(field_issue("string_type", name, STRING));
            None
        }
    }
}

fn list_field(value: Option<&Json>, name: &str, issues: &mut Vec<Issue>) -> Option<Vec<Json>> {
    match value {
        None | Some(Json::Null) => None,
        Some(Json::List(items)) => Some(items.clone()),
        Some(_) => {
            issues.push(field_issue("list_type", name, LIST));
            None
        }
    }
}

fn dict_field(value: Option<&Json>, issues: &mut Vec<Issue>) -> Option<Json> {
    match value {
        None | Some(Json::Null) => None,
        Some(Json::Object(_)) => value.cloned(),
        Some(_) => {
            issues.push(field_issue("dict_type", "completion_response", DICTIONARY));
            None
        }
    }
}
