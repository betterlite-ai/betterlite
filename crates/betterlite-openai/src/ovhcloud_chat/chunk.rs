use betterlite_prompts::arguments::json::{Json, Object};
use serde::Serialize;

use crate::openai_base_config::Headers;

use super::error::OvhCloudError;
use super::{CHUNK_OBJECT, JSON_CONTENT_TYPE};

const ERROR_PREFIX: &str = "OVHCloud Error: ";
const UNKNOWN_ERROR: &str = "Unknown error";
const DEFAULT_STATUS: u16 = 400;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OvhCloudChunk {
    pub id: String,
    pub object: &'static str,
    pub created: i64,
    pub model: Option<String>,
    pub usage: Option<Json>,
    pub choices: Vec<Json>,
}

// chunk_parser reads the error key first, then the choices, then the chunk's own id, created and model
pub fn chunk_parser(chunk: &Object) -> Result<OvhCloudChunk, OvhCloudError> {
    if let Some(error) = chunk.get("error") {
        return Err(error_chunk(error));
    }
    let Some(Json::List(choices)) = chunk.get("choices") else {
        return Err(match chunk.get("choices") {
            None => missing_key(chunk, "choices"),
            Some(_) => not_a_list(),
        });
    };
    let choices = with_reasoning(choices)?;
    let (id, created, model) = chunk_keys(chunk)?;
    Ok(OvhCloudChunk {
        id,
        object: CHUNK_OBJECT,
        created,
        model,
        usage: chunk.get("usage").cloned(),
        choices,
    })
}

fn with_reasoning(choices: &[Json]) -> Result<Vec<Json>, OvhCloudError> {
    let mut projected = Vec::with_capacity(choices.len());
    for choice in choices {
        let Json::Object(mut fields) = choice.clone() else {
            return Err(shape("choices"));
        };
        match fields.get_mut("delta") {
            Some(Json::Object(delta)) => copy_reasoning(delta),
            Some(_) => return Err(shape("delta")),
            None => {}
        }
        projected.push(Json::Object(fields));
    }
    Ok(projected)
}

// LiteLLM copies reasoning into reasoning_content only when reasoning is set and reasoning_content is null
fn copy_reasoning(delta: &mut Object) {
    let Some(reasoning) = delta
        .get("reasoning")
        .cloned()
        .filter(|value| !matches!(value, Json::Null))
    else {
        return;
    };
    if delta
        .get("reasoning_content")
        .is_some_and(|current| !matches!(current, Json::Null))
    {
        return;
    }
    delta.insert(String::from("reasoning_content"), reasoning);
}

fn chunk_keys(chunk: &Object) -> Result<(String, i64, Option<String>), OvhCloudError> {
    let id = match chunk.get("id") {
        None => return Err(missing_key(chunk, "id")),
        Some(Json::Text(text)) => text.clone(),
        Some(_) => return Err(shape("id")),
    };
    let created = match chunk.get("created") {
        None => return Err(missing_key(chunk, "created")),
        Some(Json::Number(number)) => number.as_i64().ok_or_else(|| shape("created"))?,
        Some(_) => return Err(shape("created")),
    };
    let model = match chunk.get("model") {
        None => return Err(missing_key(chunk, "model")),
        Some(Json::Null) => None,
        Some(Json::Text(text)) => Some(text.clone()),
        Some(_) => return Err(shape("model")),
    };
    Ok((id, created, model))
}

fn error_chunk(error: &Json) -> OvhCloudError {
    let Json::Object(fields) = error else {
        return shape("error");
    };
    let message = match fields.get("message") {
        None | Some(Json::Null) => String::from(UNKNOWN_ERROR),
        Some(Json::Text(text)) => text.clone(),
        Some(_) => return shape("error.message"),
    };
    let status_code = match fields.get("code") {
        None => Some(DEFAULT_STATUS),
        Some(Json::Null) => None,
        Some(Json::Number(number)) => number
            .as_u64()
            .and_then(|value| u16::try_from(value).ok())
            .map(Some)
            .unwrap_or_else(|| shape("error.code")),
        Some(_) => return shape("error.code"),
    };
    OvhCloudError {
        message: format!("{ERROR_PREFIX}{message}"),
        status_code,
        headers: json_headers(),
    }
}

fn missing_key(chunk: &Object, key: &str) -> OvhCloudError {
    OvhCloudError {
        message: format!(
            "KeyError: {key}, Got unexpected response from CometAPI: {}",
            serde_json::to_string(chunk).unwrap_or_default()
        ),
        status_code: Some(DEFAULT_STATUS),
        headers: json_headers(),
    }
}

// LiteLLM iterates a non-list choices value and lets the TypeError escape; naming the field is the 4xx it belongs in
fn not_a_list() -> OvhCloudError {
    OvhCloudError {
        message: String::from("the OVHCloud stream chunk field choices is not a list"),
        status_code: Some(DEFAULT_STATUS),
        headers: json_headers(),
    }
}

fn shape(field: &'static str) -> OvhCloudError {
    OvhCloudError {
        message: format!("the OVHCloud stream chunk field {field} has an unexpected shape"),
        status_code: None,
        headers: Headers::new(),
    }
}

fn json_headers() -> Headers {
    Headers::from([(
        String::from("Content-Type"),
        String::from(JSON_CONTENT_TYPE),
    )])
}
