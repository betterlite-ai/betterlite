use crate::provider_chunk_decode::chunk::{
    DecodedChunk, ProviderChunk, as_object, at, first, text_at,
};
use crate::provider_chunk_decode::error::ProviderChunkError;
use crate::provider_chunk_decode::handlers::rejected_text;
use crate::stream_chunk_shaping::extras::strip_sse_data_from_chunk;
use crate::stream_chunk_shaping::records::JsonValue;

pub(super) fn fill_codestral(
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
    let Some(choice) = at(parsed, "choices").and_then(first).and_then(as_object) else {
        return Ok(());
    };
    let text = choice
        .get("delta")
        .and_then(|delta| text_at(delta, "content"))
        .unwrap_or_default();
    text.clone_into(&mut decoded.text);
    if let Some(reason) = choice.get("finish_reason").and_then(JsonValue::as_str) {
        decoded.is_finished = true;
        decoded.finish_reason = Some(reason.to_owned());
    }
    decoded.logprobs = choice
        .get("logprobs")
        .filter(|logprobs| !matches!(logprobs, JsonValue::Null))
        .cloned();
    decoded.original = Some(ProviderChunk::Json(Box::new(parsed.clone())).openai_chunk()?);
    Ok(())
}
