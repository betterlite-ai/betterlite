use betterlite_models::decisions::Json;
use betterlite_streaming::stream_chunk_shaping::records::GenericChunk;
use indexmap::IndexMap;

use crate::completion::error::CompletionError;

const NOT_AN_INTEGER: CompletionError = CompletionError::FieldType {
    field: "index",
    expected: "an integer",
};

fn chunk_index(chunk: &IndexMap<String, Json>) -> Result<i64, CompletionError> {
    match chunk.get("index") {
        None | Some(Json::Null) => Ok(0),
        Some(Json::Number(number)) => number.as_i64().ok_or(NOT_AN_INTEGER),
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(NOT_AN_INTEGER)
        }
    }
}

// the chunk is finished even without a stop_reason, because LiteLLM substitutes an empty string and then tests it against None
pub fn chunk_parser(chunk: &IndexMap<String, Json>) -> Result<GenericChunk, CompletionError> {
    let index = chunk_index(chunk)?;
    let text = match chunk.get("completion") {
        Some(Json::Text(text)) => text.clone(),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            String::new()
        }
    };
    let finish_reason = match chunk.get("stop_reason") {
        None | Some(Json::Null) => String::new(),
        Some(Json::Text(reason)) => reason.clone(),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(CompletionError::FieldType {
                field: "stop_reason",
                expected: "text or null",
            });
        }
    };
    Ok(GenericChunk {
        text,
        tool_use: None,
        is_finished: true,
        finish_reason,
        usage: None,
        index: Some(index),
        provider_specific_fields: None,
    })
}
