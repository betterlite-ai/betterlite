use crate::provider_chunk_decode::chunk::{
    DecodeState, DecodedChunk, ProviderChunk, as_array, as_object, is_null,
};
use crate::provider_chunk_decode::dispatch::finish_state;
use crate::provider_chunk_decode::error::ProviderChunkError;
use crate::provider_chunk_decode::handlers::rejected_text;
use crate::stream_chunk_shaping::extras::strip_sse_data_from_chunk;
use crate::stream_chunk_shaping::records::JsonValue;

pub(super) fn decode_codestral(
    chunk: &ProviderChunk,
    state: &mut DecodeState,
) -> Result<DecodedChunk, ProviderChunkError> {
    let mut decoded = DecodedChunk::default();
    fill_codestral(chunk, &mut decoded)?;
    finish_state(state, &decoded);
    Ok(decoded)
}

fn fill_codestral(
    chunk: &ProviderChunk,
    decoded: &mut DecodedChunk,
) -> Result<(), ProviderChunkError> {
    let ProviderChunk::Text(line) = chunk else {
        return Err(ProviderChunkError::NotAString {
            chunk: rejected_text(chunk),
        });
    };
    let Some(parsed) = codestral_line(line) else {
        return Ok(());
    };
    fill_codestral_choice(&parsed, decoded)
}

fn codestral_line(line: &str) -> Option<JsonValue> {
    let stripped = strip_sse_data_from_chunk(Some(line)).unwrap_or(line).trim();
    match stripped {
        "" | "[DONE]" => None,
        payload => serde_json::from_str(payload).ok(),
    }
}

fn fill_codestral_choice(
    parsed: &JsonValue,
    decoded: &mut DecodedChunk,
) -> Result<(), ProviderChunkError> {
    let Some(choice) = as_object(parsed)
        .and_then(|fields| fields.get("choices"))
        .and_then(as_array)
        .and_then(|choices| choices.first())
        .and_then(as_object)
    else {
        return Ok(());
    };
    let text = choice
        .get("delta")
        .and_then(as_object)
        .and_then(|delta| delta.get("content"))
        .and_then(JsonValue::as_str)
        .unwrap_or_default();
    text.clone_into(&mut decoded.text);
    if let Some(reason) = choice
        .get("finish_reason")
        .filter(|reason| !is_null(reason))
        .and_then(JsonValue::as_str)
    {
        decoded.is_finished = true;
        decoded.finish_reason = Some(reason.to_owned());
    }
    decoded.logprobs = choice
        .get("logprobs")
        .filter(|logprobs| !is_null(logprobs))
        .cloned();
    decoded.original = Some(ProviderChunk::Json(Box::new(parsed.clone())).openai_chunk()?);
    Ok(())
}
