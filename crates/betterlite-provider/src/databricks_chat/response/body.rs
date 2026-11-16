use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::{Json, Object};
use serde_json::Value;

use crate::databricks_chat::error::DatabricksChatError;
use crate::databricks_chat::response::ResponseInput;
use crate::databricks_chat::response::reply;
use crate::databricks_chat::response::usage::usage_from;

// DatabricksResponse(**raw_response.json()): a body that is not an object cannot be spread
pub(super) fn parse_body(input: &ResponseInput<'_>) -> Result<Object, DatabricksChatError> {
    let parsed: Json = match serde_json::from_str(input.body) {
        Ok(parsed) => parsed,
        Err(error) => return Err(parse_error(input, &error.to_string())),
    };
    match parsed {
        Json::Object(fields) => Ok(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(parse_error(input, "the response body is not a JSON object"))
        }
    }
}

fn parse_error(input: &ResponseInput<'_>, error: &str) -> DatabricksChatError {
    DatabricksChatError::Response {
        status_code: input.status_code,
        message: format!(
            "Unable to get json response - {error}, Original Response: {}",
            input.body
        ),
        headers: input.headers.clone(),
    }
}

pub(super) fn response_model(
    input: &ResponseInput<'_>,
    body: &Object,
) -> Result<String, DatabricksChatError> {
    let provider = match input.custom_llm_provider {
        Some(name) if !name.is_empty() => name,
        None | Some(_) => "databricks",
    };
    let model = match body.get("model") {
        None | Some(Json::Null) => "",
        Some(Json::Text(text)) => text.as_str(),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(wrong_type("model", "a string"));
        }
    };
    Ok(format!("{provider}/{model}"))
}

pub(super) fn usage_of(body: &Object) -> Result<reply::Usage, DatabricksChatError> {
    match body.get("usage") {
        Some(Json::Object(fields)) => usage_from(fields),
        None => Err(missing_key("usage")),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(wrong_type("usage", "an object"))
        }
    }
}

pub(super) fn choices_of(body: &Object) -> Result<&[Json], DatabricksChatError> {
    match body.get("choices") {
        Some(Json::List(items)) => Ok(items),
        None => Err(missing_key("choices")),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            Err(wrong_type("choices", "a list"))
        }
    }
}

pub(super) fn required_text<'value>(
    body: &'value Object,
    name: &str,
) -> Result<&'value str, DatabricksChatError> {
    match body.get(name) {
        Some(Json::Text(text)) => Ok(text),
        None => Err(missing_key(name)),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(wrong_type(name, "a string"))
        }
    }
}

pub(super) fn required_integer(body: &Object, name: &str) -> Result<i64, DatabricksChatError> {
    match body.get(name) {
        Some(Json::Number(number)) => match number.as_i64() {
            Some(value) => Ok(value),
            None => Err(wrong_type(name, "an integer")),
        },
        None => Err(missing_key(name)),
        Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(wrong_type(name, "an integer"))
        }
    }
}

pub(super) fn native_fields(
    native: BTreeMap<String, Value>,
) -> Result<Option<Object>, DatabricksChatError> {
    if native.is_empty() {
        return Ok(None);
    }
    let mut fields = Object::new();
    for (name, value) in native {
        let value = serde_json::from_value(value).map_err(DatabricksChatError::Decode)?;
        fields.insert(name, value);
    }
    Ok(Some(fields))
}

pub(super) fn missing_key(name: &str) -> DatabricksChatError {
    DatabricksChatError::Shape {
        message: format!("the Databricks response is missing its {name}"),
    }
}

pub(super) fn wrong_type(name: &str, expected: &str) -> DatabricksChatError {
    DatabricksChatError::Shape {
        message: format!("the {name} of the Databricks response must be {expected}"),
    }
}
