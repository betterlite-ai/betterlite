use crate::provider_chunk_decode::chunk::{
    DecodeState, DecodedChunk, ProviderChunk, as_array, as_bool, as_object, is_null, json_text,
};
use crate::provider_chunk_decode::error::ProviderChunkError;
use crate::stream_chunk_shaping::extras::strip_sse_data_from_chunk;
use crate::stream_chunk_shaping::records::{JsonObject, JsonValue, StreamChunk, Usage};

// NLP Cloud's dolphin models stream the whole answer again in every line.
#[must_use]
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
    if let Some(JsonValue::String(text)) = fields
        .get("token")
        .and_then(as_object)
        .and_then(|token| token.get("text"))
    {
        text.clone_into(&mut decoded.text);
    }
    let details_reason = fields
        .get("details")
        .and_then(as_object)
        .and_then(|details| details.get("finish_reason"))
        .and_then(JsonValue::as_str)
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
    let parsed =
        serde_json::from_str(line).map_err(|source| ProviderChunkError::ParseResponse {
            chunk: line.to_owned(),
            source,
        })?;
    let generated = as_object(&parsed)
        .and_then(|fields| fields.get("generated_text"))
        .and_then(JsonValue::as_str)
        .ok_or_else(|| ProviderChunkError::RejectedResponse {
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

pub fn handle_aleph_alpha_chunk(
    chunk: &ProviderChunk,
    decoded: &mut DecodedChunk,
) -> Result<(), ProviderChunkError> {
    let line = chunk.text()?;
    let parsed =
        serde_json::from_str(line).map_err(|source| ProviderChunkError::ParseResponse {
            chunk: line.to_owned(),
            source,
        })?;
    let completion = as_object(&parsed)
        .and_then(|fields| fields.get("completions"))
        .and_then(as_array)
        .and_then(|completions| completions.first())
        .and_then(as_object)
        .and_then(|completion| completion.get("completion"))
        .and_then(JsonValue::as_str)
        .ok_or_else(|| ProviderChunkError::RejectedResponse {
            chunk: line.to_owned(),
        })?;
    completion.clone_into(&mut decoded.text);
    decoded.is_finished = true;
    decoded.finish_reason = Some("stop".to_owned());
    Ok(())
}

pub fn handle_replicate_chunk(
    chunk: &ProviderChunk,
    decoded: &mut DecodedChunk,
) -> Result<(), ProviderChunkError> {
    let fields = object_of(chunk)?;
    if let Some(output) = fields.get("output") {
        output
            .as_str()
            .ok_or(ProviderChunkError::InvalidField { field: "output" })?
            .clone_into(&mut decoded.text);
    }
    match fields.get("status") {
        Some(status) => {
            if status.as_str() == Some("succeeded") {
                decoded.is_finished = true;
                decoded.finish_reason = Some("stop".to_owned());
            }
        }
        None => reject_replicate(chunk, fields)?,
    }
    Ok(())
}

fn reject_replicate(chunk: &ProviderChunk, fields: &JsonObject) -> Result<(), ProviderChunkError> {
    if fields.get("error").is_some_and(|error| !is_null(error)) {
        return Err(ProviderChunkError::RejectedResponse {
            chunk: rejected_text(chunk),
        });
    }
    Ok(())
}

pub fn handle_openai_chat_chunk(chunk: &StreamChunk, decoded: &mut DecodedChunk) {
    decoded.original = Some(chunk.clone());
    decoded.usage.clone_from(&chunk.usage);
    let Some(choice) = chunk.choices.first() else {
        return;
    };
    decoded.text = choice.delta.content.clone().unwrap_or_default();
    decoded.logprobs.clone_from(&choice.logprobs);
    let Some(reason) = choice
        .finish_reason
        .clone()
        .filter(|reason| !reason.is_empty())
    else {
        return;
    };
    decoded.is_finished = true;
    if let Some(JsonValue::Bool(flag)) = chunk.hidden_params.get("is_finished") {
        decoded.is_finished = *flag;
    }
    decoded.finish_reason = Some(reason);
}

pub fn handle_text_completion_chunk(
    chunk: &ProviderChunk,
    decoded: &mut DecodedChunk,
    keep_usage: bool,
) -> Result<(), ProviderChunkError> {
    let Ok(fields) = object_of(chunk) else {
        return Ok(());
    };
    let choice = fields
        .get("choices")
        .and_then(as_array)
        .and_then(|choices| choices.first())
        .and_then(as_object);
    if let Some(first) = choice {
        decoded.text = string_field(first, "text")?.unwrap_or_default();
        if let Some(reason) = string_field(first, "finish_reason")? {
            decoded.is_finished = true;
            decoded.finish_reason = Some(reason);
        }
    }
    if keep_usage {
        decoded.usage = usage_counts(fields.get("usage"))?;
    }
    Ok(())
}

pub fn handle_triton_stream(
    chunk: &ProviderChunk,
    decoded: &mut DecodedChunk,
) -> Result<(), ProviderChunkError> {
    if let ProviderChunk::Json(value) = chunk
        && let JsonValue::Object(fields) = value.as_ref()
    {
        return triton_decoded(fields, decoded);
    }
    let line = chunk.text()?;
    if !line.contains("text_output") {
        return Ok(());
    }
    let stripped = strip_sse_data_from_chunk(Some(line)).unwrap_or(line).trim();
    let parsed =
        serde_json::from_str(stripped).map_err(|source| ProviderChunkError::ParseResponse {
            chunk: line.to_owned(),
            source,
        })?;
    let JsonValue::Object(fields) = parsed else {
        return Err(ProviderChunkError::UnexpectedChunk {
            expected: "a JSON object",
        });
    };
    triton_decoded(&fields, decoded)
}

fn triton_decoded(
    fields: &JsonObject,
    decoded: &mut DecodedChunk,
) -> Result<(), ProviderChunkError> {
    decoded.text = string_field(fields, "text_output")?.unwrap_or_default();
    decoded.finish_reason = string_field(fields, "stop_reason")?;
    if let Some(flag) = fields.get("is_finished").and_then(as_bool) {
        decoded.is_finished = flag;
    }
    Ok(())
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
    let Some(counted) = counts.filter(|value| !is_null(value)) else {
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
