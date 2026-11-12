use super::error::CometApiError;
use super::{CHUNK_OBJECT, JSON_CONTENT_TYPE};
use crate::openai_base_config::Headers;
use betterlite_prompts::arguments::json::{Json, Object};
use serde::Serialize;
const ERROR_PREFIX: &str = "CometAPI Error: ";
const UNKNOWN_ERROR: &str = "Unknown error";
const MISSING_ERROR_CODE: u16 = 400;
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CometApiChunk {
    pub id: String,
    pub object: &'static str,
    pub created: i64,
    pub model: Option<String>,
    pub usage: Option<Json>,
    pub choices: Vec<Json>,
}
// chunk_parser visits the choices before it reads the chunk's own keys, and an error key wins over both
pub fn chunk_parser(chunk: &Object) -> Result<CometApiChunk, CometApiError> {
    if let Some(error) = chunk.get("error") {
        return Err(error_chunk(error)?);
    }
    let Some(Json::List(choices)) = chunk.get("choices") else {
        return Err(match chunk.get("choices") {
            None => missing("choices"),
            Some(_) => shape("choices"),
        });
    };
    let projected = projected_choices(choices)?;
    let (id, created, model) = chunk_keys(chunk)?;
    Ok(CometApiChunk {
        id,
        object: CHUNK_OBJECT,
        created,
        model,
        usage: chunk.get("usage").cloned(),
        choices: projected,
    })
}
fn projected_choices(choices: &[Json]) -> Result<Vec<Json>, CometApiError> {
    let mut projected = Vec::with_capacity(choices.len());
    for choice in choices {
        let Json::Object(mut fields) = choice.clone() else {
            return Err(shape("choices"));
        };
        copy_reasoning(&mut fields)?;
        projected.push(Json::Object(fields));
    }
    Ok(projected)
}
fn chunk_keys(chunk: &Object) -> Result<(String, i64, Option<String>), CometApiError> {
    let id = match chunk.get("id") {
        None => return Err(missing("id")),
        Some(Json::Text(text)) => text.clone(),
        Some(_) => return Err(shape("id")),
    };
    let created = match chunk.get("created") {
        None => return Err(missing("created")),
        Some(Json::Number(number)) => number.as_i64().ok_or_else(|| shape("created"))?,
        Some(_) => return Err(shape("created")),
    };
    let model = match chunk.get("model") {
        None => return Err(missing("model")),
        Some(Json::Null) => None,
        Some(Json::Text(text)) => Some(text.clone()),
        Some(_) => return Err(shape("model")),
    };
    Ok((id, created, model))
}
// a present reasoning key is copied even when it is null, which overwrites an earlier reasoning_content
fn copy_reasoning(fields: &mut Object) -> Result<(), CometApiError> {
    let Some(delta) = fields.get_mut("delta") else {
        return Ok(());
    };
    let Json::Object(delta_fields) = delta else {
        return Err(shape("delta"));
    };
    if let Some(reasoning) = delta_fields.get("reasoning").cloned() {
        delta_fields.insert("reasoning_content".to_owned(), reasoning);
    }
    Ok(())
}
fn error_chunk(error: &Json) -> Result<CometApiError, CometApiError> {
    let Json::Object(fields) = error else {
        return Err(shape("error"));
    };
    let message = match fields.get("message") {
        None | Some(Json::Null) => String::from(UNKNOWN_ERROR),
        Some(Json::Text(text)) => text.clone(),
        Some(_) => return Err(shape("error.message")),
    };
    let status_code = match fields.get("code") {
        None => Some(MISSING_ERROR_CODE),
        Some(Json::Null) => None,
        Some(Json::Number(number)) => number
            .as_u64()
            .and_then(|value| u16::try_from(value).ok())
            .map(Some)
            .ok_or_else(|| shape("error.code"))?,
        Some(_) => return Err(shape("error.code")),
    };
    Ok(CometApiError {
        message: format!("{ERROR_PREFIX}{message}"),
        status_code,
        headers: json_headers(),
    })
}
fn shape(field: &'static str) -> CometApiError {
    CometApiError {
        message: format!("the CometAPI stream chunk field {field} has an unexpected shape"),
        status_code: None,
        headers: Headers::new(),
    }
}
fn missing(field: &'static str) -> CometApiError {
    CometApiError {
        message: format!("the CometAPI stream chunk is missing the {field} field"),
        status_code: Some(MISSING_ERROR_CODE),
        headers: json_headers(),
    }
}
fn json_headers() -> Headers {
    Headers::from([(
        String::from("Content-Type"),
        String::from(JSON_CONTENT_TYPE),
    )])
}
