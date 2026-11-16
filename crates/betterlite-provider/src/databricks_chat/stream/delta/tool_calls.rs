use betterlite_prompts::arguments::json::{Json, Object};
use serde_json::Number;

use crate::databricks_chat::error::DatabricksChatError;
use crate::databricks_chat::function::{NormalizedFunction, normalize_function};
use crate::databricks_chat::stream::delta::copy_extra;

const CALL_NAMED: [&str; 4] = ["id", "type", "function", "index"];
const CUSTOM_NAMED: [&str; 4] = ["id", "type", "custom", "index"];
const PAYLOAD_NAMED: [&str; 2] = ["name", "input"];

// tool_calls: a list is coerced call by call, a falsy non-list stays as it came
pub(super) fn normalize(value: Option<&Json>) -> Result<Json, DatabricksChatError> {
    let Some(value) = value else {
        return Ok(Json::Null);
    };
    match value {
        Json::Null => Ok(Json::Null),
        Json::List(items) if items.is_empty() => Ok(Json::List(Vec::new())),
        Json::List(items) => {
            let mut calls = Vec::with_capacity(items.len());
            let mut next_index = 0_i64;
            for item in items {
                calls.push(Json::Object(delta_call(item, &mut next_index)?));
            }
            Ok(Json::List(calls))
        }
        other if !other.is_truthy() => Ok(other.clone()),
        _ => Err(DatabricksChatError::wrong_type(
            "tool_calls of a Databricks stream delta",
            "a list",
        )),
    }
}

// Delta assigns an index only to a call that has none, counting from zero
fn delta_call(item: &Json, next_index: &mut i64) -> Result<Object, DatabricksChatError> {
    let Some(fields) = item.as_object() else {
        return Err(DatabricksChatError::wrong_type(
            "a tool call of a Databricks stream delta",
            "an object",
        ));
    };
    let index = index_of(fields, next_index)?;
    if is_custom(fields) {
        custom_call(fields, index)
    } else {
        function_call(fields, index)
    }
}

fn index_of(fields: &Object, next_index: &mut i64) -> Result<i64, DatabricksChatError> {
    match fields.get("index") {
        None | Some(Json::Null) => {
            let assigned = *next_index;
            *next_index += 1;
            Ok(assigned)
        }
        Some(Json::Number(number)) => number
            .as_i64()
            .ok_or_else(|| DatabricksChatError::wrong_type("index of a tool call", "an integer")),
        Some(Json::Text(text)) => text
            .parse::<i64>()
            .map_err(|_| DatabricksChatError::wrong_type("index of a tool call", "an integer")),
        Some(_) => Err(DatabricksChatError::wrong_type(
            "index of a tool call",
            "an integer",
        )),
    }
}

fn is_custom(fields: &Object) -> bool {
    fields.get("type").and_then(Json::as_str) == Some("custom")
        || !matches!(fields.get("custom"), None | Some(Json::Null))
}

fn function_call(fields: &Object, index: i64) -> Result<Object, DatabricksChatError> {
    let function = match fields.get("function") {
        Some(Json::Object(function)) => function,
        None => return Err(DatabricksChatError::missing("tool call", "function")),
        Some(_) => {
            return Err(DatabricksChatError::wrong_type(
                "function of a tool call",
                "an object",
            ));
        }
    };
    let NormalizedFunction { arguments, name } = normalize_function(function)?;
    let mut normalized = Object::new();
    normalized.insert(String::from("arguments"), Json::Text(arguments));
    normalized.insert(String::from("name"), name.map_or(Json::Null, Json::Text));
    let mut call = Object::new();
    call.insert(String::from("id"), id_of(fields)?);
    call.insert(String::from("type"), kind_of(fields)?);
    call.insert(String::from("function"), Json::Object(normalized));
    call.insert(String::from("index"), Json::Number(Number::from(index)));
    copy_extra(fields, &mut call, &CALL_NAMED);
    Ok(call)
}

fn id_of(fields: &Object) -> Result<Json, DatabricksChatError> {
    match fields.get("id") {
        None | Some(Json::Null) => Ok(Json::Null),
        Some(Json::Text(text)) => Ok(Json::Text(text.clone())),
        Some(_) => Err(DatabricksChatError::wrong_type(
            "id of a tool call",
            "a string",
        )),
    }
}

fn kind_of(fields: &Object) -> Result<Json, DatabricksChatError> {
    match fields.get("type") {
        None | Some(Json::Null) => Ok(Json::Text(String::from("function"))),
        Some(Json::Text(text)) => Ok(Json::Text(text.clone())),
        Some(_) => Err(DatabricksChatError::wrong_type(
            "type of a tool call",
            "a string",
        )),
    }
}

// ChatCompletionDeltaCustomToolCall: the payload keeps name and input, null when unset
fn custom_call(fields: &Object, index: i64) -> Result<Object, DatabricksChatError> {
    let custom = match fields.get("custom") {
        Some(Json::Object(custom)) => custom,
        None => return Err(DatabricksChatError::missing("tool call", "custom")),
        Some(_) => {
            return Err(DatabricksChatError::wrong_type(
                "custom of a tool call",
                "an object",
            ));
        }
    };
    let mut call = Object::new();
    call.insert(String::from("id"), id_of(fields)?);
    call.insert(String::from("type"), custom_kind_of(fields)?);
    call.insert(
        String::from("custom"),
        Json::Object(custom_payload(custom)?),
    );
    call.insert(String::from("index"), Json::Number(Number::from(index)));
    copy_extra(fields, &mut call, &CUSTOM_NAMED);
    if matches!(call.get("function"), Some(Json::Null)) {
        call.shift_remove("function");
    }
    Ok(call)
}

fn custom_kind_of(fields: &Object) -> Result<Json, DatabricksChatError> {
    match fields.get("type") {
        None | Some(Json::Null) => Ok(Json::Null),
        Some(Json::Text(text)) => Ok(Json::Text(text.clone())),
        Some(_) => Err(DatabricksChatError::wrong_type(
            "type of a tool call",
            "a string",
        )),
    }
}

fn custom_payload(fields: &Object) -> Result<Object, DatabricksChatError> {
    let mut payload = Object::new();
    for name in ["name", "input"] {
        payload.insert(name.to_owned(), payload_field(fields, name)?);
    }
    copy_extra(fields, &mut payload, &PAYLOAD_NAMED);
    Ok(payload)
}

fn payload_field(fields: &Object, name: &str) -> Result<Json, DatabricksChatError> {
    match fields.get(name) {
        None | Some(Json::Null) => Ok(Json::Null),
        Some(Json::Text(text)) => Ok(Json::Text(text.clone())),
        Some(_) => Err(DatabricksChatError::wrong_type(
            &format!("{name} of a custom tool call"),
            "a string",
        )),
    }
}
