use std::str;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::error::{EstimateError, Issue};

const REQUIRED: &str = "Field required";
const STRING: &str = "Input should be a valid string";
const LIST: &str = "Input should be a valid list";
const DICTIONARY: &str = "Input should be a valid dictionary";
const OBJECT: &str = "Input should be a valid dictionary or object to extract fields from";
const JSON_DECODE: &str = "JSON decode error";

// SpendCalculateRequest's three optional fields, each already narrowed to the type pydantic declares.
#[derive(Debug)]
pub struct SpendCalculateRequest {
    pub model: Option<String>,
    pub messages: Option<Vec<Json>>,
    pub completion_response: Option<Json>,
}

// An empty body and malformed JSON answer like FastAPI's request validation; every field is optional.
pub fn decode_spend_estimate(bytes: &[u8]) -> Result<SpendCalculateRequest, EstimateError> {
    let fields = parse_body(bytes)?;
    let mut issues = Vec::new();
    let model = narrow(&fields, "model", &mut issues);
    let messages = narrow(&fields, "messages", &mut issues);
    let completion_response = narrow(&fields, "completion_response", &mut issues);
    if !issues.is_empty() {
        return Err(EstimateError::Request(issues));
    }
    Ok(SpendCalculateRequest {
        model,
        messages,
        completion_response,
    })
}

fn parse_body(bytes: &[u8]) -> Result<IndexMap<String, Json>, EstimateError> {
    if bytes.is_empty() {
        return Err(EstimateError::Request(vec![body_issue(
            "missing", REQUIRED,
        )]));
    }
    let parsed = str::from_utf8(bytes)
        .ok()
        .and_then(|text| serde_json::from_str::<Json>(text).ok());
    let Some(parsed) = parsed else {
        return Err(EstimateError::Request(vec![body_issue(
            "json_invalid",
            JSON_DECODE,
        )]));
    };
    let Json::Object(fields) = parsed else {
        return Err(EstimateError::Request(vec![body_issue(
            "model_attributes_type",
            OBJECT,
        )]));
    };
    Ok(fields)
}

// The type each optional field declares, with the issue pydantic reports when a value doesn't fit it
trait FieldValue: Sized {
    const KIND: &'static str;
    const MESSAGE: &'static str;

    fn from_json(value: &Json) -> Option<Self>;
}

impl FieldValue for String {
    const KIND: &'static str = "string_type";
    const MESSAGE: &'static str = STRING;

    fn from_json(value: &Json) -> Option<Self> {
        match value {
            Json::Text(text) => Some(text.clone()),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
        }
    }
}

impl FieldValue for Vec<Json> {
    const KIND: &'static str = "list_type";
    const MESSAGE: &'static str = LIST;

    fn from_json(value: &Json) -> Option<Self> {
        match value {
            Json::List(items) => Some(items.clone()),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => None,
        }
    }
}

impl FieldValue for Json {
    const KIND: &'static str = "dict_type";
    const MESSAGE: &'static str = DICTIONARY;

    fn from_json(value: &Self) -> Option<Self> {
        match value {
            object @ Self::Object(_) => Some(object.clone()),
            Self::Null | Self::Bool(_) | Self::Number(_) | Self::Text(_) | Self::List(_) => None,
        }
    }
}

fn narrow<T>(
    fields: &IndexMap<String, Json>,
    name: &'static str,
    issues: &mut Vec<Issue>,
) -> Option<T>
where
    T: FieldValue,
{
    match fields.get(name) {
        None | Some(Json::Null) => None,
        Some(value) => {
            let Some(narrowed) = T::from_json(value) else {
                issues.push(field_issue(T::KIND, name, T::MESSAGE));
                return None;
            };
            Some(narrowed)
        }
    }
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
