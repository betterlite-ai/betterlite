use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::request_form::values::{FormParts, PartValue};
use serde_json::{Map, Value};
use thiserror::Error;

use crate::api::request_form::{RequestFormError, decode_request_form};

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ParsedRequestData {
    pub query_params: Option<Value>,
    pub custom_body: Option<Value>,
    pub stream: Option<bool>,
}

#[derive(Debug, Error)]
pub enum ParsedRequestError {
    #[error("the request body is not a JSON object")]
    BodyNotObject,
    #[error("the {field} field of the request body has the wrong type")]
    FieldType { field: &'static str },
    #[error(transparent)]
    Form(#[from] RequestFormError),
}

#[derive(Debug, Clone, Copy)]
enum EnvelopeFallback {
    WholeBody,
    KeepAbsent,
}

enum JsonBody {
    Object(Map<String, Value>),
    NotObject,
    Invalid,
}

// LiteLLM's _parse_request_data_by_content_type: the URL query is the fallback only when no known content type matched
pub fn parse_request_by_content_type(
    content_type: &str,
    body: &[u8],
    query: &[(String, String)],
    limits: &ApiLimits,
) -> Result<ParsedRequestData, ParsedRequestError> {
    if content_type.contains("application/json") {
        return match json_body(body) {
            JsonBody::Object(object) => envelope(&object, EnvelopeFallback::KeepAbsent),
            JsonBody::NotObject => Err(ParsedRequestError::BodyNotObject),
            JsonBody::Invalid => Ok(ParsedRequestData::default()),
        };
    }
    if content_type.contains("multipart/form-data") {
        return Ok(match json_body(body) {
            JsonBody::Object(object) => envelope(&object, EnvelopeFallback::WholeBody)?,
            JsonBody::NotObject | JsonBody::Invalid => ParsedRequestData::default(),
        });
    }
    if content_type.contains("application/x-www-form-urlencoded") {
        return urlencoded_fields(body, content_type, limits);
    }
    Ok(ParsedRequestData {
        query_params: query_params_object(query),
        custom_body: None,
        stream: None,
    })
}

// a misconfigured client can send JSON under a multipart content type, so the bytes are tried as JSON first
fn json_body(body: &[u8]) -> JsonBody {
    match serde_json::from_slice::<Value>(body) {
        Ok(Value::Object(object)) => JsonBody::Object(object),
        Ok(_other) => JsonBody::NotObject,
        Err(_error) => JsonBody::Invalid,
    }
}

fn envelope(
    object: &Map<String, Value>,
    fallback: EnvelopeFallback,
) -> Result<ParsedRequestData, ParsedRequestError> {
    let query_params = match object.get("query_params") {
        None | Some(Value::Null) => None,
        Some(value) if value.is_object() => Some(value.clone()),
        Some(_other) => {
            return Err(ParsedRequestError::FieldType {
                field: "query_params",
            });
        }
    };
    let stream = match object.get("stream") {
        None | Some(Value::Null) => None,
        Some(Value::Bool(value)) => Some(*value),
        Some(_other) => return Err(ParsedRequestError::FieldType { field: "stream" }),
    };
    let custom_body = match object.get("custom_body") {
        Some(Value::Null) | None
            if matches!(fallback, EnvelopeFallback::WholeBody) && !object.is_empty() =>
        {
            Some(Value::Object(object.clone()))
        }
        Some(Value::Null) | None => None,
        Some(value) if value.is_object() => Some(value.clone()),
        Some(_other) => {
            return Err(ParsedRequestError::FieldType {
                field: "custom_body",
            });
        }
    };
    Ok(ParsedRequestData {
        query_params,
        custom_body,
        stream,
    })
}

fn urlencoded_fields(
    body: &[u8],
    content_type: &str,
    limits: &ApiLimits,
) -> Result<ParsedRequestData, ParsedRequestError> {
    let parts = match decode_request_form(body, content_type.as_bytes(), limits) {
        Ok(parts) => parts,
        Err(RequestFormError::ContentType) => return Ok(ParsedRequestData::default()),
        Err(error) => return Err(ParsedRequestError::Form(error)),
    };
    Ok(ParsedRequestData {
        query_params: last_text_field(&parts, "query_params").map(Value::String),
        custom_body: last_text_field(&parts, "custom_body").map(Value::String),
        stream: None,
    })
}

// Starlette's FormData keeps the last value sent under a repeated name
fn last_text_field(parts: &FormParts, name: &str) -> Option<String> {
    parts.items.iter().rev().find_map(|part| match &part.value {
        PartValue::Text(value) if part.name == name => Some(value.clone()),
        PartValue::Text(_) | PartValue::Upload(_) => None,
    })
}

fn query_params_object(query: &[(String, String)]) -> Option<Value> {
    if query.is_empty() {
        return None;
    }
    let mut object = Map::new();
    for (name, value) in query {
        object.insert(name.clone(), Value::String(value.clone()));
    }
    Some(Value::Object(object))
}
