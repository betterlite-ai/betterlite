use crate::provider_chunk_decode::chunk::{DecodeState, ProviderChunk};
use crate::provider_chunk_decode::creator::original::{
    apply_original_choice, fill_completion, fill_decoded, fill_tool_use, hidden_params_of,
    normalize_original, sync_finish,
};
use crate::provider_chunk_decode::dispatch::decode_provider_chunk;
use crate::provider_chunk_decode::error::ProviderChunkError;
use crate::stream_chunk_shaping::ChunkShaper;
use crate::stream_chunk_shaping::creation::{
    copy_provider_specific_fields, model_response_creator, set_model_id,
};
use crate::stream_chunk_shaping::decision::return_processed_chunk_logic;
use crate::stream_chunk_shaping::records::{
    ChunkResponseObject, StreamChunk, StreamDelta, item_count,
};
use crate::stream_chunk_shaping::usage_totals::record_usage_only_chunk;

mod fields;
mod original;

pub fn chunk_creator(
    shaper: &mut ChunkShaper,
    state: &mut DecodeState,
    chunk: &ProviderChunk,
) -> Result<Option<StreamChunk>, ProviderChunkError> {
    if let Some(id) = chunk.response_id() {
        shaper.response_id = Some(id.to_owned());
    }
    let hidden = hidden_params_of(chunk);
    let mut model_response = model_response_creator(shaper, None, hidden.as_ref())?;
    let decoded = decode_provider_chunk(shaper.provider.as_deref(), chunk, state)?;
    if decoded.early_return {
        return Ok(decoded.original);
    }
    fill_decoded(chunk, state, shaper, &decoded, &mut model_response);
    let mut response_obj = ChunkResponseObject {
        original_chunk: decoded.original.clone(),
        provider_specific_fields: decoded.provider_specific_fields.clone(),
    };
    let mut completion_obj = StreamDelta::default();
    fill_completion(&mut completion_obj, &decoded);
    let processed = match prepare_original(shaper, state, &mut response_obj, model_response) {
        Prepared::Continue(response) => response,
        Prepared::Emit(ready) => return Ok(Some(ready)),
        Prepared::Skip => return Ok(None),
    };
    if completion_obj
        .tool_calls
        .as_ref()
        .is_some_and(|calls| item_count(calls) > 0)
    {
        fill_tool_use(shaper, state, &mut completion_obj)?;
    }
    sync_finish(shaper, state);
    let result = return_processed_chunk_logic(
        shaper,
        &mut completion_obj,
        processed,
        &response_obj,
        state.repeated_chunk_limit,
    )?;
    state.sent_first_chunk = shaper.progress.sent_first_chunk;
    Ok(result)
}

enum Prepared {
    Continue(StreamChunk),
    Emit(StreamChunk),
    Skip,
}

fn prepare_original(
    shaper: &mut ChunkShaper,
    state: &mut DecodeState,
    response_obj: &mut ChunkResponseObject,
    mut model_response: StreamChunk,
) -> Prepared {
    let Some(original) = &mut response_obj.original_chunk else {
        return Prepared::Continue(model_response);
    };
    normalize_original(original);
    set_model_id(shaper, &original.id, &mut model_response);
    copy_provider_specific_fields(original, &mut model_response);
    if !original.choices.is_empty() {
        apply_original_choice(&mut model_response, original);
        return Prepared::Continue(model_response);
    }
    state.sent_first_chunk = shaper.progress.sent_first_chunk;
    if state.options.send_stream_usage {
        model_response.choices.clear();
        return Prepared::Emit(model_response);
    }
    record_usage_only_chunk(shaper, model_response);
    Prepared::Skip
}
