use crate::provider_chunk_decode::chunk::{
    DecodeState, DecodedChunk, ProviderChunk, as_array, as_object,
};
use crate::provider_chunk_decode::dispatch::finish_state;
use crate::provider_chunk_decode::error::ProviderChunkError;
use crate::provider_chunk_decode::handlers::openai::{
    handle_openai_chat_chunk, handle_text_completion_chunk, handle_triton_stream,
};
use crate::provider_chunk_decode::handlers::{
    handle_aleph_alpha_chunk, handle_nlp_cloud_chunk, handle_predibase_chunk,
    handle_replicate_chunk,
};
use crate::provider_chunk_decode::vertex::handle_vertex_chunk;
use crate::stream_chunk_shaping::records::JsonValue;

pub(super) fn decode_replicate(
    chunk: &ProviderChunk,
    state: &mut DecodeState,
) -> Result<DecodedChunk, ProviderChunkError> {
    let mut decoded = DecodedChunk::default();
    handle_replicate_chunk(chunk, &mut decoded)?;
    finish_state(state, &decoded);
    Ok(decoded)
}

pub(super) fn decode_predibase(
    chunk: &ProviderChunk,
    state: &mut DecodeState,
) -> Result<DecodedChunk, ProviderChunkError> {
    let mut decoded = DecodedChunk::default();
    handle_predibase_chunk(chunk, &mut decoded)?;
    finish_state(state, &decoded);
    Ok(decoded)
}

pub(super) fn decode_vllm(chunk: &ProviderChunk) -> Result<DecodedChunk, ProviderChunkError> {
    let mut decoded = DecodedChunk::default();
    fill_vllm(chunk, &mut decoded)?;
    Ok(decoded)
}

fn fill_vllm(chunk: &ProviderChunk, decoded: &mut DecodedChunk) -> Result<(), ProviderChunkError> {
    let JsonValue::Array(items) = chunk.json()? else {
        return Err(ProviderChunkError::UnexpectedChunk {
            expected: "a vllm output list",
        });
    };
    let text = items
        .first()
        .and_then(as_object)
        .and_then(|item| item.get("outputs"))
        .and_then(as_array)
        .and_then(|outputs| outputs.first())
        .and_then(as_object)
        .and_then(|output| output.get("text"))
        .and_then(JsonValue::as_str)
        .ok_or(ProviderChunkError::MissingField {
            field: "outputs[0].text",
        })?;
    text.clone_into(&mut decoded.text);
    Ok(())
}

pub(super) fn decode_aleph_alpha(
    chunk: &ProviderChunk,
    state: &mut DecodeState,
) -> Result<DecodedChunk, ProviderChunkError> {
    let mut decoded = DecodedChunk::default();
    handle_aleph_alpha_chunk(chunk, &mut decoded)?;
    finish_state(state, &decoded);
    Ok(decoded)
}

pub(super) fn decode_nlp_cloud(
    chunk: &ProviderChunk,
    state: &mut DecodeState,
) -> Result<DecodedChunk, ProviderChunkError> {
    let mut decoded = DecodedChunk::default();
    match handle_nlp_cloud_chunk(chunk, state, &mut decoded) {
        Ok(()) => {
            finish_state(state, &decoded);
            Ok(decoded)
        }
        Err(error) => nlp_cloud_failure(state, error),
    }
}

fn nlp_cloud_failure(
    state: &mut DecodeState,
    error: ProviderChunkError,
) -> Result<DecodedChunk, ProviderChunkError> {
    if state.received_finish_reason.is_some() {
        return Err(error);
    }
    if state.sent_first_chunk {
        state.received_finish_reason = Some("stop".to_owned());
        return Ok(DecodedChunk::default());
    }
    Err(ProviderChunkError::UnknownStream)
}

pub(super) fn decode_vertex(
    chunk: &ProviderChunk,
    state: &mut DecodeState,
) -> Result<DecodedChunk, ProviderChunkError> {
    let mut decoded = DecodedChunk::default();
    handle_vertex_chunk(chunk, state, &mut decoded)?;
    Ok(decoded)
}

pub(super) fn decode_petals(state: &mut DecodeState) -> Result<DecodedChunk, ProviderChunkError> {
    let mut decoded = DecodedChunk::default();
    fill_petals(state, &mut decoded)?;
    Ok(decoded)
}

fn fill_petals(
    state: &mut DecodeState,
    decoded: &mut DecodedChunk,
) -> Result<(), ProviderChunkError> {
    let stream = state.completion_stream.take().unwrap_or_default();
    if stream.is_empty() {
        if state.received_finish_reason.is_some() {
            return Err(ProviderChunkError::StopIteration);
        }
        state.received_finish_reason = Some("stop".to_owned());
    }
    decoded.text = stream.chars().take(30).collect();
    state.completion_stream = Some(stream.chars().skip(30).collect());
    Ok(())
}

pub(super) fn decode_triton(
    chunk: &ProviderChunk,
    state: &mut DecodeState,
) -> Result<DecodedChunk, ProviderChunkError> {
    let mut decoded = DecodedChunk::default();
    handle_triton_stream(chunk, &mut decoded)?;
    finish_state(state, &decoded);
    Ok(decoded)
}

pub(super) fn decode_text_openai(
    chunk: &ProviderChunk,
    state: &mut DecodeState,
) -> Result<DecodedChunk, ProviderChunkError> {
    let mut decoded = DecodedChunk::default();
    handle_text_completion_chunk(chunk, &mut decoded, true)?;
    finish_state(state, &decoded);
    Ok(decoded)
}

pub(super) fn decode_azure_text(
    chunk: &ProviderChunk,
    state: &mut DecodeState,
) -> Result<DecodedChunk, ProviderChunkError> {
    let mut decoded = DecodedChunk::default();
    handle_text_completion_chunk(chunk, &mut decoded, false)?;
    finish_state(state, &decoded);
    Ok(decoded)
}

pub(super) fn decode_cached(
    chunk: &ProviderChunk,
    state: &mut DecodeState,
) -> Result<DecodedChunk, ProviderChunkError> {
    let mut decoded = DecodedChunk::default();
    fill_cached(chunk, &mut decoded)?;
    if decoded.is_finished {
        state
            .received_finish_reason
            .clone_from(&decoded.finish_reason);
    }
    Ok(decoded)
}

fn fill_cached(
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

pub(super) fn decode_openai(
    provider: Option<&str>,
    chunk: &ProviderChunk,
    state: &mut DecodeState,
) -> Result<DecodedChunk, ProviderChunkError> {
    let mut decoded = DecodedChunk::default();
    fill_openai(provider, chunk, state, &mut decoded)?;
    Ok(decoded)
}

fn fill_openai(
    provider: Option<&str>,
    chunk: &ProviderChunk,
    state: &mut DecodeState,
    decoded: &mut DecodedChunk,
) -> Result<(), ProviderChunkError> {
    if matches!(provider, Some("azure" | "azure_ai"))
        && let Some(model) = chunk.model_name()
    {
        model.clone_into(&mut state.model);
    }
    let stream = chunk.openai_chunk()?;
    handle_openai_chat_chunk(&stream, decoded);
    state
        .intermittent_finish_reason
        .clone_from(&decoded.finish_reason);
    if decoded.is_finished {
        if decoded.finish_reason.as_deref() == Some("error") {
            return Err(ProviderChunkError::StreamingFailure {
                provider: provider.unwrap_or_default().to_owned(),
            });
        }
        state
            .received_finish_reason
            .clone_from(&decoded.finish_reason);
    }
    decoded.system_fingerprint = Some(stream.system_fingerprint.clone());
    decoded.response_system_fingerprint = Some(stream.system_fingerprint);
    Ok(())
}
