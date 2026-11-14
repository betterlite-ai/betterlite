use betterlite_models::decisions::Json;
use betterlite_streaming::stream_chunk_shaping::extras::strip_sse_data_from_chunk;
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

fn chunk_finish_reason(chunk: &IndexMap<String, Json>) -> Result<String, CompletionError> {
    match chunk.get("stop_reason") {
        None | Some(Json::Null) => Ok(String::new()),
        Some(Json::Text(reason)) => Ok(reason.clone()),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(CompletionError::FieldType {
                field: "stop_reason",
                expected: "text or null",
            })
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
    Ok(GenericChunk {
        text,
        tool_use: None,
        is_finished: true,
        finish_reason: chunk_finish_reason(chunk)?,
        usage: None,
        index: Some(index),
        provider_specific_fields: None,
    })
}

// B2214's BaseModelResponseIterator._handle_string_chunk over one transport line, then chunk_parser
pub fn decode_stream_line(line: &str) -> Result<GenericChunk, CompletionError> {
    let empty = GenericChunk {
        index: Some(0),
        ..GenericChunk::default()
    };
    if line.contains("[DONE]") {
        return Ok(GenericChunk {
            is_finished: true,
            finish_reason: "stop".to_owned(),
            ..empty
        });
    }
    let stripped = strip_sse_data_from_chunk(Some(line)).unwrap_or(line);
    match serde_json::from_str::<Json>(stripped) {
        Ok(Json::Object(fields)) if !fields.is_empty() => chunk_parser(&fields),
        Ok(Json::Object(_) | Json::Null) | Err(_) => Ok(empty),
        Ok(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(CompletionError::ChunkNotObject)
        }
    }
}
