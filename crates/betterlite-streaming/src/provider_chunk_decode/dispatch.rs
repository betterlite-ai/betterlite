use crate::provider_chunk_decode::chunk::{
    DecodeState, DecodedChunk, ProviderChunk, ProviderKind, as_array, as_object, is_null,
};
use crate::provider_chunk_decode::error::ProviderChunkError;
use crate::provider_chunk_decode::handlers::{
    handle_aleph_alpha_chunk, handle_nlp_cloud_chunk, handle_openai_chat_chunk,
    handle_predibase_chunk, handle_replicate_chunk, handle_text_completion_chunk,
    handle_triton_stream, rejected_text,
};
use crate::provider_chunk_decode::vertex::handle_vertex_chunk;
use crate::stream_chunk_shaping::extras::{
    generic_chunk_has_known_fields, strip_sse_data_from_chunk,
};
use crate::stream_chunk_shaping::records::{GenericChunk, JsonValue, StreamChunk, item_count};

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

fn finish_state(state: &mut DecodeState, decoded: &DecodedChunk) {
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

fn decode_replicate(
    chunk: &ProviderChunk,
    state: &mut DecodeState,
) -> Result<DecodedChunk, ProviderChunkError> {
    let mut decoded = DecodedChunk::default();
    handle_replicate_chunk(chunk, &mut decoded)?;
    finish_state(state, &decoded);
    Ok(decoded)
}

fn decode_predibase(
    chunk: &ProviderChunk,
    state: &mut DecodeState,
) -> Result<DecodedChunk, ProviderChunkError> {
    let mut decoded = DecodedChunk::default();
    handle_predibase_chunk(chunk, &mut decoded)?;
    finish_state(state, &decoded);
    Ok(decoded)
}

fn decode_vllm(chunk: &ProviderChunk) -> Result<DecodedChunk, ProviderChunkError> {
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

fn decode_aleph_alpha(
    chunk: &ProviderChunk,
    state: &mut DecodeState,
) -> Result<DecodedChunk, ProviderChunkError> {
    let mut decoded = DecodedChunk::default();
    handle_aleph_alpha_chunk(chunk, &mut decoded)?;
    finish_state(state, &decoded);
    Ok(decoded)
}

fn decode_nlp_cloud(
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

fn decode_vertex(
    chunk: &ProviderChunk,
    state: &mut DecodeState,
) -> Result<DecodedChunk, ProviderChunkError> {
    let mut decoded = DecodedChunk::default();
    handle_vertex_chunk(chunk, state, &mut decoded)?;
    Ok(decoded)
}

fn decode_petals(state: &mut DecodeState) -> Result<DecodedChunk, ProviderChunkError> {
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

fn decode_triton(
    chunk: &ProviderChunk,
    state: &mut DecodeState,
) -> Result<DecodedChunk, ProviderChunkError> {
    let mut decoded = DecodedChunk::default();
    handle_triton_stream(chunk, &mut decoded)?;
    finish_state(state, &decoded);
    Ok(decoded)
}

fn decode_text_openai(
    chunk: &ProviderChunk,
    state: &mut DecodeState,
) -> Result<DecodedChunk, ProviderChunkError> {
    let mut decoded = DecodedChunk::default();
    handle_text_completion_chunk(chunk, &mut decoded, true)?;
    finish_state(state, &decoded);
    Ok(decoded)
}

fn decode_azure_text(
    chunk: &ProviderChunk,
    state: &mut DecodeState,
) -> Result<DecodedChunk, ProviderChunkError> {
    let mut decoded = DecodedChunk::default();
    handle_text_completion_chunk(chunk, &mut decoded, false)?;
    finish_state(state, &decoded);
    Ok(decoded)
}

fn decode_codestral(
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

fn decode_cached(
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

fn decode_openai(
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
