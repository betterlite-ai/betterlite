use serde::Deserialize;

use crate::provider_chunk_decode::chunk::{DecodedChunk, ProviderChunk, as_array, as_object};
use crate::provider_chunk_decode::error::ProviderChunkError;
use crate::provider_chunk_decode::handlers::{object_of, parse_line, string_field, usage_counts};
use crate::stream_chunk_shaping::extras::strip_sse_data_from_chunk;
use crate::stream_chunk_shaping::records::{JsonObject, JsonValue, StreamChunk};
use crate::stream_chunk_shaping::tokens::mapped_finish_reason;

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
    decoded.finish_reason = Some(mapped_finish_reason(&reason).to_owned());
}

pub fn handle_text_completion_chunk(
    chunk: &ProviderChunk,
    decoded: &mut DecodedChunk,
) -> Result<(), ProviderChunkError> {
    let fields = object_of(chunk).ok();
    fill_text_choice(fields, decoded)?;
    decoded.usage = usage_counts(fields.and_then(|object| object.get("usage")))?;
    Ok(())
}

pub fn handle_azure_text_chunk(
    chunk: &ProviderChunk,
    decoded: &mut DecodedChunk,
) -> Result<(), ProviderChunkError> {
    fill_text_choice(object_of(chunk).ok(), decoded)?;
    Ok(())
}

fn fill_text_choice(
    fields: Option<&JsonObject>,
    decoded: &mut DecodedChunk,
) -> Result<(), ProviderChunkError> {
    let Some(choice) = fields
        .and_then(|object| object.get("choices"))
        .and_then(as_array)
        .and_then(<[JsonValue]>::first)
        .and_then(as_object)
    else {
        return Ok(());
    };
    decoded.text = string_field(choice, "text")?.unwrap_or_default();
    if let Some(reason) = string_field(choice, "finish_reason")? {
        decoded.is_finished = true;
        decoded.finish_reason = Some(reason);
    }
    Ok(())
}

// The keys of a triton stream object the reader looks at.
#[derive(Deserialize)]
struct TritonStream {
    text_output: Option<String>,
    stop_reason: Option<String>,
    is_finished: Option<bool>,
}

pub fn handle_triton_stream(
    chunk: &ProviderChunk,
    decoded: &mut DecodedChunk,
) -> Result<(), ProviderChunkError> {
    let data: TritonStream = if let ProviderChunk::Json(value) = chunk
        && matches!(value.as_ref(), JsonValue::Object(_))
    {
        serde_json::from_value(serde_json::to_value(value)?)?
    } else {
        let line = chunk.text()?;
        if !line.contains("text_output") {
            return Ok(());
        }
        let stripped = strip_sse_data_from_chunk(Some(line)).unwrap_or(line).trim();
        parse_line(stripped, line)?
    };
    decoded.text = data.text_output.unwrap_or_default();
    decoded.finish_reason = data.stop_reason;
    if let Some(flag) = data.is_finished {
        decoded.is_finished = flag;
    }
    Ok(())
}
