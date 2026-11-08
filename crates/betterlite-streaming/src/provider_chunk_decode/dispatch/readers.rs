use crate::provider_chunk_decode::chunk::{
    DecodeState, DecodedChunk, ProviderChunk, as_array, as_object, text_at,
};
use crate::provider_chunk_decode::error::ProviderChunkError;
use crate::provider_chunk_decode::handlers::openai::handle_openai_chat_chunk;
use crate::stream_chunk_shaping::records::JsonValue;

pub(super) fn fill_petals(state: &mut DecodeState) -> Result<DecodedChunk, ProviderChunkError> {
    let mut decoded = DecodedChunk::default();
    let stream = state.completion_stream.take().unwrap_or_default();
    if stream.is_empty() {
        if state.received_finish_reason.is_some() {
            return Err(ProviderChunkError::StopIteration);
        }
        state.received_finish_reason = Some("stop".to_owned());
    }
    decoded.text = stream.chars().take(30).collect();
    state.completion_stream = Some(stream.chars().skip(30).collect());
    Ok(decoded)
}

pub(super) fn fill_openai(
    provider: Option<&str>,
    chunk: &ProviderChunk,
    state: &mut DecodeState,
) -> Result<DecodedChunk, ProviderChunkError> {
    let mut decoded = DecodedChunk::default();
    if matches!(provider, Some("azure" | "azure_ai"))
        && let Some(model) = chunk.model_name()
    {
        model.clone_into(&mut state.model);
    }
    let stream = chunk.openai_chunk()?;
    handle_openai_chat_chunk(&stream, &mut decoded);
    state
        .intermittent_finish_reason
        .clone_from(&decoded.finish_reason);
    if decoded.is_finished {
        state
            .received_finish_reason
            .clone_from(&decoded.finish_reason);
    }
    decoded.system_fingerprint = Some(stream.system_fingerprint.clone());
    decoded.response_system_fingerprint = Some(stream.system_fingerprint);
    Ok(decoded)
}

pub(super) fn fill_vllm(
    chunk: &ProviderChunk,
    decoded: &mut DecodedChunk,
) -> Result<(), ProviderChunkError> {
    let items = as_array(chunk.json()?).ok_or(ProviderChunkError::UnexpectedChunk {
        expected: "a vllm output list",
    })?;
    let text = items
        .first()
        .and_then(|item| as_object(item)?.get("outputs"))
        .and_then(as_array)
        .and_then(<[JsonValue]>::first)
        .and_then(|output| text_at(output, "text"))
        .ok_or(ProviderChunkError::MissingField {
            field: "outputs[0].text",
        })?;
    text.clone_into(&mut decoded.text);
    Ok(())
}

pub(super) fn fill_cached(
    chunk: &ProviderChunk,
    decoded: &mut DecodedChunk,
) -> Result<(), ProviderChunkError> {
    let stream = chunk.openai_chunk()?;
    let choice = stream.choices.first();
    let finish_reason = choice.and_then(|first| first.finish_reason.clone());
    decoded.text = choice
        .and_then(|first| first.delta.content.clone())
        .unwrap_or_default();
    decoded.is_finished = finish_reason.is_some();
    decoded.finish_reason.clone_from(&finish_reason);
    decoded.tool_calls = choice.and_then(|first| first.delta.tool_calls.clone());
    decoded.original = Some(stream.clone());
    decoded.system_fingerprint = Some(stream.system_fingerprint.clone());
    Ok(())
}
