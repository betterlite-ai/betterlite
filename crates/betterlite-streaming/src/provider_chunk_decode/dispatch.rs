use crate::provider_chunk_decode::chunk::{
    DecodeState, DecodedChunk, ProviderChunk, ProviderKind, as_object,
};
use crate::provider_chunk_decode::dispatch::codestral::fill_codestral;
use crate::provider_chunk_decode::dispatch::readers::{
    fill_cached, fill_openai, fill_petals, fill_vllm,
};
use crate::provider_chunk_decode::error::ProviderChunkError;
use crate::provider_chunk_decode::handlers::aleph::handle_aleph_alpha_chunk;
use crate::provider_chunk_decode::handlers::openai::{
    handle_azure_text_chunk, handle_text_completion_chunk, handle_triton_stream,
};
use crate::provider_chunk_decode::handlers::replicate::handle_replicate_chunk;
use crate::provider_chunk_decode::handlers::{handle_nlp_cloud_chunk, handle_predibase_chunk};
use crate::provider_chunk_decode::vertex::handle_vertex_chunk;
use crate::stream_chunk_shaping::extras::generic_chunk_has_known_fields;
use crate::stream_chunk_shaping::records::{GenericChunk, JsonValue, StreamChunk, item_count};

mod codestral;
mod readers;

type Fill = fn(&ProviderChunk, &mut DecodedChunk) -> Result<(), ProviderChunkError>;

const READERS: [(&str, Fill); 8] = [
    ("predibase", handle_predibase_chunk),
    ("vllm", fill_vllm),
    ("aleph_alpha", handle_aleph_alpha_chunk),
    ("triton", handle_triton_stream),
    ("text-completion-openai", handle_text_completion_chunk),
    ("text-completion-codestral", fill_codestral),
    ("azure_text", handle_azure_text_chunk),
    ("cached_response", fill_cached),
];

pub fn decode_provider_chunk(
    provider: Option<&str>,
    chunk: &ProviderChunk,
    state: &mut DecodeState,
) -> Result<DecodedChunk, ProviderChunkError> {
    if matches!(state.kind, ProviderKind::Custom)
        && !is_generic_shape(chunk)
        && chunk.is_openai_chunk()
    {
        return decode_custom(chunk, state);
    }
    if is_generic_shape(chunk) || matches!(state.kind, ProviderKind::Custom) {
        return decode_generic(chunk, state);
    }
    if provider == Some("replicate") || state.model == "replicate" {
        return decoded_from(state, chunk, handle_replicate_chunk);
    }
    decode_named(provider, chunk, state)
}

fn decode_named(
    provider: Option<&str>,
    chunk: &ProviderChunk,
    state: &mut DecodeState,
) -> Result<DecodedChunk, ProviderChunkError> {
    if provider == Some("vertex_ai") && !chunk.is_openai_chunk() {
        return handle_vertex_chunk(chunk, state);
    }
    if provider == Some("nlp_cloud") {
        return nlp_cloud_step(chunk, state);
    }
    if provider == Some("petals") {
        return fill_petals(state);
    }
    match provider.and_then(reader_of) {
        Some(fill) => decoded_from(state, chunk, fill),
        None => fill_openai(provider, chunk, state),
    }
}

fn reader_of(provider: &str) -> Option<Fill> {
    READERS
        .iter()
        .find(|(name, _)| *name == provider)
        .map(|(_, fill)| *fill)
}

#[must_use]
pub fn is_generic_shape(chunk: &ProviderChunk) -> bool {
    match chunk {
        ProviderChunk::Generic(_) => true,
        ProviderChunk::Json(value) => as_object(value).is_some_and(generic_chunk_has_known_fields),
        ProviderChunk::Text(_) | ProviderChunk::Bytes(_) => false,
    }
}

pub(super) fn finish_state(state: &mut DecodeState, decoded: &DecodedChunk) {
    if decoded.is_finished {
        state
            .received_finish_reason
            .clone_from(&decoded.finish_reason);
    }
}

fn decode_custom(
    chunk: &ProviderChunk,
    state: &mut DecodeState,
) -> Result<DecodedChunk, ProviderChunkError> {
    let mut decoded = DecodedChunk::default();
    let mut stream = chunk.openai_chunk()?;
    let has_content = first_choice_has_content(&stream);
    if state.received_finish_reason.is_some() && !has_content {
        return Err(ProviderChunkError::StopIteration);
    }
    let finish = stream
        .choices
        .first_mut()
        .and_then(|choice| choice.finish_reason.take_if(|reason| !reason.is_empty()));
    if let Some(reason) = finish {
        state.received_finish_reason = Some(reason);
        if !has_content {
            fill_early(&mut decoded, None);
            return Ok(decoded);
        }
    }
    fill_early(&mut decoded, Some(stream));
    Ok(decoded)
}

fn fill_early(decoded: &mut DecodedChunk, original: Option<StreamChunk>) {
    decoded.early_return = true;
    decoded.original = original;
}

fn first_choice_has_content(stream: &StreamChunk) -> bool {
    stream.choices.first().is_some_and(|choice| {
        choice
            .delta
            .content
            .as_deref()
            .is_some_and(|text| !text.is_empty())
            || choice
                .delta
                .tool_calls
                .as_ref()
                .is_some_and(|calls| item_count(calls) > 0)
    })
}

fn decode_generic(
    chunk: &ProviderChunk,
    state: &mut DecodeState,
) -> Result<DecodedChunk, ProviderChunkError> {
    let generic = generic_of(chunk)?;
    let has_content =
        !generic.text.is_empty() || generic.tool_use.is_some() || generic.usage.is_some();
    if state.received_finish_reason.is_some()
        && !has_content
        && !carries_provider_fields(chunk, &generic)
    {
        return Err(ProviderChunkError::StopIteration);
    }
    let mut decoded = DecodedChunk::default();
    fill_generic(&mut decoded, &generic);
    if generic.is_finished {
        state.received_finish_reason = Some(generic.finish_reason.clone());
    }
    if !generic.finish_reason.is_empty() {
        state.intermittent_finish_reason = Some(generic.finish_reason.clone());
    }
    Ok(decoded)
}

fn fill_generic(decoded: &mut DecodedChunk, generic: &GenericChunk) {
    decoded.text.clone_from(&generic.text);
    decoded.is_finished = generic.is_finished;
    decoded.finish_reason = Some(generic.finish_reason.clone());
    decoded.usage.clone_from(&generic.usage);
    decoded.tool_use.clone_from(&generic.tool_use);
    decoded.tool_calls = generic
        .tool_use
        .as_ref()
        .map(|tool_use| JsonValue::Array(vec![tool_use.clone()]));
    decoded
        .provider_specific_fields
        .clone_from(&generic.provider_specific_fields);
}

fn generic_of(chunk: &ProviderChunk) -> Result<GenericChunk, ProviderChunkError> {
    match chunk {
        ProviderChunk::Generic(generic) => Ok(generic.as_ref().clone()),
        ProviderChunk::Json(value) => {
            let plain = serde_json::to_value(value.as_ref()).map_err(ProviderChunkError::from)?;
            serde_json::from_value(plain).map_err(ProviderChunkError::from)
        }
        ProviderChunk::Text(_) | ProviderChunk::Bytes(_) => {
            Err(ProviderChunkError::UnexpectedChunk {
                expected: "a generic streaming chunk",
            })
        }
    }
}

fn carries_provider_fields(chunk: &ProviderChunk, generic: &GenericChunk) -> bool {
    match chunk {
        ProviderChunk::Json(value) => {
            as_object(value).is_some_and(|fields| fields.contains_key("provider_specific_fields"))
        }
        ProviderChunk::Generic(_) => generic.provider_specific_fields.is_some(),
        ProviderChunk::Text(_) | ProviderChunk::Bytes(_) => false,
    }
}

fn decoded_from(
    state: &mut DecodeState,
    chunk: &ProviderChunk,
    fill: Fill,
) -> Result<DecodedChunk, ProviderChunkError> {
    let mut decoded = DecodedChunk::default();
    fill(chunk, &mut decoded)?;
    finish_state(state, &decoded);
    Ok(decoded)
}

fn nlp_cloud_step(
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
