use betterlite_prompts::arguments::json::{Json, Object};
use serde::Serialize;

use crate::databricks_chat::error::DatabricksChatError;
use crate::databricks_chat::response::usage::usage_from;
use crate::databricks_chat::stream::choice::transform;
use crate::databricks_chat::wire::{encode_value, render};
use crate::ollama_chat::ids::IdSource;

mod choice;
mod citation;
mod delta;
mod tests;

pub use choice::StreamChoice;
pub use delta::Delta;

// ModelResponseStream as DatabricksChatResponseIterator builds it for one chunk
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StreamChunk {
    pub id: String,
    pub created: i64,
    pub model: Option<String>,
    pub object: String,
    pub choices: Vec<StreamChoice>,
    pub provider_specific_fields: Option<Json>,
    pub usage: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
}

// DatabricksChatResponseIterator: the chunk parser keeps the last json-mode function name
#[derive(Debug, Default)]
pub struct ChunkParser {
    json_mode: bool,
    last_function_name: Option<String>,
}

impl ChunkParser {
    #[must_use]
    pub const fn new(json_mode: bool) -> Self {
        Self {
            json_mode,
            last_function_name: None,
        }
    }

    pub fn parse(
        &mut self,
        chunk: &Json,
        ids: &dyn IdSource,
    ) -> Result<StreamChunk, DatabricksChatError> {
        let Some(fields) = chunk.as_object() else {
            return Err(DatabricksChatError::shape(
                "a Databricks stream chunk must be an object",
            ));
        };
        let rendered = render(chunk);
        let choices = self.translate_choices(fields, &rendered)?;
        Ok(StreamChunk {
            id: id_of(fields, &rendered, ids)?,
            created: created_of(fields, &rendered)?,
            model: model_of(fields, &rendered)?,
            object: String::from("chat.completion.chunk"),
            choices,
            provider_specific_fields: None,
            usage: usage_of(fields)?,
            service_tier: service_tier_of(fields),
        })
    }

    // chunk["choices"]: a missing key is the 400 KeyError, any other shape is a 500
    fn translate_choices(
        &mut self,
        fields: &Object,
        rendered: &str,
    ) -> Result<Vec<StreamChoice>, DatabricksChatError> {
        let items = match fields.get("choices") {
            Some(Json::List(items)) => items,
            None => return Err(DatabricksChatError::chunk_key("choices", rendered)),
            Some(_) => {
                return Err(DatabricksChatError::wrong_type(
                    "choices of a Databricks stream chunk",
                    "a list",
                ));
            }
        };
        let mut choices = Vec::with_capacity(items.len());
        for item in items {
            choices.push(transform(item, self, rendered)?);
        }
        Ok(choices)
    }
}

fn id_of(
    fields: &Object,
    rendered: &str,
    ids: &dyn IdSource,
) -> Result<String, DatabricksChatError> {
    match fields.get("id") {
        None => Err(DatabricksChatError::chunk_key("id", rendered)),
        Some(Json::Null) => Ok(ids.next_id()),
        Some(Json::Text(text)) => Ok(text.clone()),
        Some(_) => Err(DatabricksChatError::wrong_type(
            "id of a Databricks stream chunk",
            "a string",
        )),
    }
}

// created: a null one needs the current time, which a transform without a clock cannot read
fn created_of(fields: &Object, rendered: &str) -> Result<i64, DatabricksChatError> {
    match fields.get("created") {
        None => Err(DatabricksChatError::chunk_key("created", rendered)),
        Some(Json::Number(number)) => number.as_i64().ok_or_else(|| {
            DatabricksChatError::wrong_type("created of a Databricks stream chunk", "an integer")
        }),
        Some(Json::Text(text)) => text.parse::<i64>().map_err(|_| {
            DatabricksChatError::wrong_type("created of a Databricks stream chunk", "an integer")
        }),
        Some(Json::Null) => Err(DatabricksChatError::shape(
            "the created of a Databricks stream chunk is null and no clock is available",
        )),
        Some(_) => Err(DatabricksChatError::wrong_type(
            "created of a Databricks stream chunk",
            "an integer",
        )),
    }
}

fn model_of(fields: &Object, rendered: &str) -> Result<Option<String>, DatabricksChatError> {
    match fields.get("model") {
        None => Err(DatabricksChatError::chunk_key("model", rendered)),
        Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(_) => Err(DatabricksChatError::wrong_type(
            "model of a Databricks stream chunk",
            "a string",
        )),
    }
}

// usage: a dict is normalized the way Usage(**dict) validates it, any other value passes through
fn usage_of(fields: &Object) -> Result<Option<Json>, DatabricksChatError> {
    match fields.get("usage") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(block)) => Ok(Some(encode_value(&usage_from(block)?)?)),
        Some(value) => Ok(Some(value.clone())),
    }
}

fn service_tier_of(fields: &Object) -> Option<String> {
    match fields.get("service_tier") {
        Some(Json::Text(text)) if !text.is_empty() => Some(text.clone()),
        _ => None,
    }
}
