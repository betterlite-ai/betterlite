use crate::provider_chunk_decode::chunk::{
    DecodeState, DecodedChunk, ProviderChunk, as_object, json_text, text_at,
};
use crate::provider_chunk_decode::error::ProviderChunkError;
use crate::stream_chunk_shaping::records::{JsonObject, JsonValue, Usage};

pub mod aleph;
pub mod openai;
pub mod replicate;

pub fn process_chunk(state: &mut DecodeState, chunk: &str) -> String {
    let trimmed = chunk.trim();
    let known = state.complete_response.trim().to_owned();
    let delta = trimmed.strip_prefix(known.as_str()).unwrap_or(trimmed);
    state.complete_response = format!("{known}{delta}");
    delta.to_owned()
}

pub fn handle_predibase_chunk(
    chunk: &ProviderChunk,
    decoded: &mut DecodedChunk,
) -> Result<(), ProviderChunkError> {
    let line = chunk.text()?;
    let Some(payload) = line.strip_prefix("data:") else {
        if line.contains("error") {
            return Err(ProviderChunkError::RejectedResponse {
                chunk: line.to_owned(),
            });
        }
        return Ok(());
    };
    let parsed =
        serde_json::from_str(payload).map_err(|source| ProviderChunkError::ParseResponse {
            chunk: line.to_owned(),
            source,
        })?;
    let JsonValue::Object(fields) = parsed else {
        return Err(ProviderChunkError::UnexpectedChunk {
            expected: "a JSON object",
        });
    };
    if let Some(text) = fields.get("token").and_then(|token| text_at(token, "text")) {
        text.clone_into(&mut decoded.text);
    }
    let details_reason = fields
        .get("details")
        .and_then(|details| text_at(details, "finish_reason"))
        .filter(|reason| !reason.is_empty());
    match details_reason {
        Some(reason) => {
            decoded.is_finished = true;
            decoded.finish_reason = Some(reason.to_owned());
            Ok(())
        }
        None => predibase_fallback(&fields, decoded),
    }
}

fn predibase_fallback(
    fields: &JsonObject,
    decoded: &mut DecodedChunk,
) -> Result<(), ProviderChunkError> {
    if fields
        .get("generated_text")
        .and_then(JsonValue::as_str)
        .is_some_and(|text| !text.is_empty())
    {
        decoded.text.clear();
        decoded.is_finished = true;
        decoded.finish_reason = Some("stop".to_owned());
        return Ok(());
    }
    if let Some(error) = fields
        .get("error")
        .and_then(JsonValue::as_str)
        .filter(|error| !error.is_empty())
    {
        return Err(ProviderChunkError::ProviderReported {
            message: error.to_owned(),
        });
    }
    Ok(())
}

pub(super) fn parse_line(line: &str) -> Result<JsonValue, ProviderChunkError> {
    serde_json::from_str(line).map_err(|source| ProviderChunkError::ParseResponse {
        chunk: line.to_owned(),
        source,
    })
}

pub fn handle_nlp_cloud_chunk(
    chunk: &ProviderChunk,
    state: &mut DecodeState,
    decoded: &mut DecodedChunk,
) -> Result<(), ProviderChunkError> {
    let line = chunk.text()?;
    if state.model.contains("dolphin") {
        fill_nlp_cloud(decoded, &process_chunk(state, line));
        return Ok(());
    }
    let parsed = parse_line(line)?;
    let generated =
        text_at(&parsed, "generated_text").ok_or_else(|| ProviderChunkError::RejectedResponse {
            chunk: line.to_owned(),
        })?;
    fill_nlp_cloud(decoded, generated);
    Ok(())
}

fn fill_nlp_cloud(decoded: &mut DecodedChunk, text: &str) {
    if text.contains("[DONE]") {
        decoded.text = text.replace("[DONE]", "");
        decoded.is_finished = true;
        decoded.finish_reason = Some("stop".to_owned());
        return;
    }
    text.clone_into(&mut decoded.text);
}

#[must_use]
pub fn rejected_text(chunk: &ProviderChunk) -> String {
    match chunk {
        ProviderChunk::Json(value) => json_text(value),
        ProviderChunk::Text(text) => text.clone(),
        ProviderChunk::Generic(_) => String::new(),
        ProviderChunk::Bytes(bytes) => format!("{bytes:?}"),
    }
}

pub fn object_of(chunk: &ProviderChunk) -> Result<&JsonObject, ProviderChunkError> {
    as_object(chunk.json()?).ok_or(ProviderChunkError::UnexpectedChunk {
        expected: "a JSON object",
    })
}

pub fn string_field(
    fields: &JsonObject,
    key: &'static str,
) -> Result<Option<String>, ProviderChunkError> {
    match fields.get(key) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::String(text)) => Ok(Some(text.clone())),
        Some(
            JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Array(_) | JsonValue::Object(_),
        ) => Err(ProviderChunkError::InvalidField { field: key }),
    }
}

// The three counters of a chunk usage, and none of the details a rebuilt Usage drops.
pub fn usage_counts(counts: Option<&JsonValue>) -> Result<Option<Usage>, ProviderChunkError> {
    let Some(counted) = counts.filter(|value| !matches!(value, JsonValue::Null)) else {
        return Ok(None);
    };
    let plain = serde_json::to_value(counted).map_err(ProviderChunkError::from)?;
    let usage: Usage = serde_json::from_value(plain).map_err(ProviderChunkError::from)?;
    Ok(Some(Usage {
        prompt_tokens: usage.prompt_tokens,
        completion_tokens: usage.completion_tokens,
        total_tokens: usage.total_tokens,
        cost: None,
        prompt_tokens_details: None,
        completion_tokens_details: None,
        extra: JsonObject::new(),
    }))
}
