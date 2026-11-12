use super::error::CometApiError;
use super::{CHUNK_OBJECT, JSON_CONTENT_TYPE};
use crate::openai_base_config::Headers;
use betterlite_prompts::arguments::json::{Json, Object};
use serde::Serialize;
use thiserror::Error;
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
#[derive(Debug, Error)]
pub enum ChunkError {
    #[error(transparent)]
    Api(#[from] CometApiError),
    #[error("the CometAPI stream chunk field {field} has an unexpected shape")]
    FieldShape { field: &'static str },
}
// chunk_parser visits the choices before it reads the chunk's own keys, and an error key wins over both
pub fn chunk_parser(chunk: &Object) -> Result<CometApiChunk, ChunkError> {
    if let Some(error) = chunk.get("error") {
        return Err(ChunkError::Api(error_chunk(error)?));
    }
    let Some(Json::List(choices)) = chunk.get("choices") else {
        return Err(match chunk.get("choices") {
            None => ChunkError::Api(missing("choices")),
            Some(_) => ChunkError::FieldShape { field: "choices" },
        });
    };
    let choices = projected_choices(choices)?;
    let (id, created, model) = chunk_keys(chunk)?;
    Ok(CometApiChunk {
        id,
        object: CHUNK_OBJECT,
        created,
        model,
        usage: chunk.get("usage").cloned(),
        choices,
    })
}
fn projected_choices(choices: &[Json]) -> Result<Vec<Json>, ChunkError> {
    let mut projected = Vec::with_capacity(choices.len());
    for choice in choices {
        let Json::Object(mut fields) = choice.clone() else {
            return Err(ChunkError::FieldShape { field: "choices" });
        };
        copy_reasoning(&mut fields)?;
        projected.push(Json::Object(fields));
    }
    Ok(projected)
}
fn chunk_keys(chunk: &Object) -> Result<(String, i64, Option<String>), ChunkError> {
    let id = match chunk.get("id") {
        None => return Err(ChunkError::Api(missing("id"))),
        Some(Json::Text(text)) => text.clone(),
        Some(_) => return Err(ChunkError::FieldShape { field: "id" }),
    };
    let created = match chunk.get("created") {
        None => return Err(ChunkError::Api(missing("created"))),
        Some(Json::Number(number)) => number
            .as_i64()
            .ok_or(ChunkError::FieldShape { field: "created" })?,
        Some(_) => return Err(ChunkError::FieldShape { field: "created" }),
    };
    let model = match chunk.get("model") {
        None => return Err(ChunkError::Api(missing("model"))),
        Some(Json::Null) => None,
        Some(Json::Text(text)) => Some(text.clone()),
        Some(_) => return Err(ChunkError::FieldShape { field: "model" }),
    };
    Ok((id, created, model))
}
// a present reasoning key is copied even when it is null, which overwrites an earlier reasoning_content
fn copy_reasoning(fields: &mut Object) -> Result<(), ChunkError> {
    let Some(delta) = fields.get_mut("delta") else {
        return Ok(());
    };
    let Json::Object(delta_fields) = delta else {
        return Err(ChunkError::FieldShape { field: "delta" });
    };
    if let Some(reasoning) = delta_fields.get("reasoning").cloned() {
        delta_fields.insert("reasoning_content".to_owned(), reasoning);
    }
    Ok(())
}
fn error_chunk(error: &Json) -> Result<CometApiError, ChunkError> {
    let Json::Object(fields) = error else {
        return Err(ChunkError::FieldShape { field: "error" });
    };
    let message = match fields.get("message") {
        None => String::from(UNKNOWN_ERROR),
        Some(Json::Text(text)) => text.clone(),
        Some(_) => {
            return Err(ChunkError::FieldShape {
                field: "error.message",
            });
        }
    };
    let status_code = match fields.get("code") {
        None => Some(MISSING_ERROR_CODE),
        Some(Json::Null) => None,
        Some(Json::Number(number)) => number
            .as_u64()
            .and_then(|value| u16::try_from(value).ok())
            .map(Some)
            .ok_or(ChunkError::FieldShape {
                field: "error.code",
            })?,
        Some(_) => {
            return Err(ChunkError::FieldShape {
                field: "error.code",
            });
        }
    };
    Ok(CometApiError {
        message: format!("{ERROR_PREFIX}{message}"),
        status_code,
        headers: json_headers(),
    })
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
