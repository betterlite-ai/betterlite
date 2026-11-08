use std::sync::Arc;

use betterlite_streaming::stream_chunk_shaping::creation::SystemStreamIdentity;
use betterlite_streaming::stream_chunk_shaping::decision::is_chunk_non_empty;
use betterlite_streaming::stream_chunk_shaping::generic::generic_chunk_has_known_fields;
use betterlite_streaming::stream_chunk_shaping::sse::strip_sse_data_from_chunk;
use betterlite_streaming::stream_chunk_shaping::tokens::check_is_function_call;
use betterlite_streaming::stream_chunk_shaping::usage_totals::calculate_total_usage;
use betterlite_streaming::stream_chunk_shaping::{ChunkShaper, ChunkShaperConfig};
use serde_json::{Value, json};

use super::{Input, StateInput, StatePatch, Step};
use crate::helpers::HelperError;

mod ops;

pub(super) fn run(input: &Input) -> Result<Vec<Value>, Box<HelperError>> {
    let mut shaper = build_shaper(&input.state);
    let mut outputs = Vec::new();
    for step in &input.steps {
        let result = run_step(&mut shaper, step, input)?;
        outputs.push(json!({"result": result, "state": snapshot(&shaper)}));
    }
    Ok(outputs)
}

fn build_shaper(state: &StateInput) -> ChunkShaper {
    let mut shaper = ChunkShaper::from(ChunkShaperConfig {
        model: state.model.clone(),
        provider: state.provider.clone(),
        merge_reasoning_in_choices: state.merge_reasoning_in_choices.unwrap_or(false),
        base_hidden_params: state.base_hidden_params.clone(),
        identity: Arc::new(SystemStreamIdentity),
    });
    apply_identity(&mut shaper, state);
    apply_progress(&mut shaper, state);
    shaper
}

fn apply_identity(shaper: &mut ChunkShaper, state: &StateInput) {
    shaper.response_id.clone_from(&state.response_id);
    shaper.created = state.created;
    shaper
        .system_fingerprint
        .clone_from(&state.system_fingerprint);
    shaper.chunks.clone_from(&state.chunks);
    shaper.repeated_messages_count = state.repeated_messages_count.unwrap_or(1);
}

fn apply_progress(shaper: &mut ChunkShaper, state: &StateInput) {
    shaper.progress.sent_first_chunk = state.sent_first_chunk.unwrap_or(false);
    shaper.progress.sent_last_chunk = state.sent_last_chunk.unwrap_or(false);
    shaper.thinking.sent_first_block = state.sent_first_thinking_block.unwrap_or(false);
    shaper.thinking.sent_last_block = state.sent_last_thinking_block.unwrap_or(false);
    shaper.holding_chunk.clone_from(&state.holding_chunk);
    shaper
        .response_uptil_now
        .clone_from(&state.response_uptil_now);
    shaper
        .received_finish_reason
        .clone_from(&state.received_finish_reason);
    shaper
        .intermittent_finish_reason
        .clone_from(&state.intermittent_finish_reason);
    shaper.tool_call = state.tool_call.unwrap_or(false);
}

fn snapshot(shaper: &ChunkShaper) -> Value {
    json!({
        "response_id": shaper.response_id,
        "created": shaper.created,
        "system_fingerprint": shaper.system_fingerprint,
        "sent_first_chunk": shaper.progress.sent_first_chunk,
        "sent_last_chunk": shaper.progress.sent_last_chunk,
        "sent_first_thinking_block": shaper.thinking.sent_first_block,
        "sent_last_thinking_block": shaper.thinking.sent_last_block,
        "holding_chunk": shaper.holding_chunk,
        "response_uptil_now": shaper.response_uptil_now,
        "received_finish_reason": shaper.received_finish_reason,
        "intermittent_finish_reason": shaper.intermittent_finish_reason,
        "tool_call": shaper.tool_call,
        "repeated_messages_count": shaper.repeated_messages_count,
        "chunk_count": shaper.chunks.len(),
    })
}

fn apply_patch(shaper: &mut ChunkShaper, patch: &StatePatch) {
    if let Some(value) = patch.sent_first_chunk {
        shaper.progress.sent_first_chunk = value;
    }
    if let Some(value) = patch.sent_last_chunk {
        shaper.progress.sent_last_chunk = value;
    }
    if let Some(value) = &patch.holding_chunk {
        shaper.holding_chunk.clone_from(value);
    }
    if let Some(value) = &patch.received_finish_reason {
        shaper.received_finish_reason = Some(value.clone());
    }
    if let Some(value) = &patch.intermittent_finish_reason {
        shaper.intermittent_finish_reason = Some(value.clone());
    }
    if let Some(value) = patch.tool_call {
        shaper.tool_call = value;
    }
    if let Some(value) = patch.sent_first_thinking_block {
        shaper.thinking.sent_first_block = value;
    }
    if let Some(value) = patch.sent_last_thinking_block {
        shaper.thinking.sent_last_block = value;
    }
}

fn run_step(
    shaper: &mut ChunkShaper,
    step: &Step,
    input: &Input,
) -> Result<Value, Box<HelperError>> {
    match step {
        Step::Creator { .. } => ops::creator(shaper, step),
        Step::SetModelId { .. } => ops::set_model_id(shaper, step),
        Step::CopyProviderFields { .. } => ops::copy_provider_fields(step),
        Step::IsChunkNonEmpty {
            completion,
            chunk,
            response,
        } => Ok(json!(is_chunk_non_empty(
            shaper, completion, chunk, response
        ))),
        Step::Returns { .. } => ops::returns(shaper, step, input),
        Step::CombineThinking { .. } => ops::combine_thinking(shaper, step),
        Step::SpecialTokens { .. } => Ok(ops::special_tokens(shaper, step)),
        Step::Repetition => ops::repetition(shaper, input),
        Step::FinishReason => ops::finish_reason(shaper),
        Step::RecordUsage { .. } => Ok(ops::record_usage(shaper, step)),
        Step::StripSse { chunk } => Ok(json!(strip_sse_data_from_chunk(chunk.as_deref()))),
        Step::GenericKnown { chunk } => Ok(json!(generic_chunk_has_known_fields(chunk))),
        Step::ConvertGeneric { .. } => ops::convert_generic(step),
        Step::TotalUsage => ops::encoded(&calculate_total_usage(&shaper.chunks)),
        Step::BackfillUsage { .. } => ops::backfill(step),
        Step::ResolveCost { .. } => Ok(ops::resolved_cost(step)),
        Step::PropagateCost { .. } => ops::propagate_cost(shaper, step, input),
        Step::CacheTokenCount { .. } => Ok(ops::cache_token_count(step)),
        Step::CoerceDetails { .. } => ops::coerce_details(step),
        Step::ProviderResponseModel { .. } => Ok(ops::provider_response_model(step)),
        Step::ProviderHiddenParams { .. } => ops::provider_hidden_params(step),
        Step::StampServiceTier { .. } => ops::stamp_service_tier(step),
        Step::CheckStreamUsage { .. } => Ok(ops::check_stream_usage(step)),
        Step::CheckFunctionCall { optional_params } => {
            Ok(json!(check_is_function_call(Some(optional_params))))
        }
        Step::MappedFinishReason { .. } => Ok(ops::mapped_finish_reason_value(step)),
        Step::SetState { patch } => {
            apply_patch(shaper, patch);
            Ok(Value::Null)
        }
    }
}
