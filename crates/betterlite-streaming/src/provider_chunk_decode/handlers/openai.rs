use crate::provider_chunk_decode::chunk::{
    DecodedChunk, ProviderChunk, as_array, as_bool, as_object,
};
use crate::provider_chunk_decode::error::ProviderChunkError;
use crate::provider_chunk_decode::handlers::{object_of, string_field, usage_counts};
use crate::stream_chunk_shaping::extras::strip_sse_data_from_chunk;
use crate::stream_chunk_shaping::records::{JsonObject, JsonValue, StreamChunk};

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
