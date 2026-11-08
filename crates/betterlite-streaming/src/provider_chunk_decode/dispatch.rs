use crate::provider_chunk_decode::chunk::{
    DecodeState, DecodedChunk, ProviderChunk, ProviderKind, as_object,
};
use crate::provider_chunk_decode::dispatch::branches::{
    decode_aleph_alpha, decode_azure_text, decode_cached, decode_nlp_cloud, decode_openai,
    decode_petals, decode_predibase, decode_replicate, decode_text_openai, decode_triton,
    decode_vertex, decode_vllm,
};
use crate::provider_chunk_decode::dispatch::codestral::decode_codestral;
use crate::provider_chunk_decode::error::ProviderChunkError;
use crate::stream_chunk_shaping::extras::generic_chunk_has_known_fields;
use crate::stream_chunk_shaping::records::{GenericChunk, JsonValue, StreamChunk, item_count};

mod branches;
mod codestral;

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
        return decode_replicate(chunk, state);
    }
    match provider {
        Some("predibase") => decode_predibase(chunk, state),
        Some("vllm") => decode_vllm(chunk),
        Some("aleph_alpha") => decode_aleph_alpha(chunk, state),
        Some("nlp_cloud") => decode_nlp_cloud(chunk, state),
        Some("vertex_ai") if !chunk.is_openai_chunk() => decode_vertex(chunk, state),
        Some("petals") => decode_petals(state),
        Some("triton") => decode_triton(chunk, state),
        Some("text-completion-openai") => decode_text_openai(chunk, state),
        Some("text-completion-codestral") => decode_codestral(chunk, state),
        Some("azure_text") => decode_azure_text(chunk, state),
        Some("cached_response") => decode_cached(chunk, state),
        Some(_) | None => decode_openai(provider, chunk, state),
    }
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
