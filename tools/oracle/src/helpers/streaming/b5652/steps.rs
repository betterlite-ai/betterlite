use std::sync::Arc;

use betterlite_streaming::stream_chunk_shaping::ChunkShaper;
use betterlite_streaming::stream_chunk_shaping::ChunkShaperConfig;
use betterlite_streaming::stream_chunk_shaping::creation::{
    SystemStreamIdentity, copy_provider_specific_fields, model_response_creator,
    provider_hidden_params, provider_response_model, set_model_id, stamp_served_service_tier,
};
use betterlite_streaming::stream_chunk_shaping::decision::{
    combine_thinking_in_choices, is_chunk_non_empty, return_processed_chunk_logic,
};
use betterlite_streaming::stream_chunk_shaping::generic::{
    convert_generic_chunk, generic_chunk_has_known_fields,
};
use betterlite_streaming::stream_chunk_shaping::sse::strip_sse_data_from_chunk;
use betterlite_streaming::stream_chunk_shaping::tokens::{
    check_is_function_call, check_send_stream_usage, check_special_tokens, finish_reason_handler,
    mapped_finish_reason, raise_on_model_repetition,
};
use betterlite_streaming::stream_chunk_shaping::usage_totals::{
    backfill_cache_usage, cache_token_count, calculate_total_usage, coerce_token_details,
    propagate_usage_cost, record_usage_only_chunk, resolve_provider_reported_cost,
};
use serde_json::{Value, json};

use super::{Input, StateInput, StatePatch, Step};
use crate::helpers::HelperError;

mod ops;

use ops::{decimal_json, encoded, failure, shape_error};

type StepResult = Result<Value, Box<HelperError>>;

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
    shaper.response_id.clone_from(&state.response_id);
    shaper.created = state.created;
    shaper
        .system_fingerprint
        .clone_from(&state.system_fingerprint);
    shaper.chunks.clone_from(&state.chunks);
    shaper.repeated_messages_count = state.repeated_messages_count.unwrap_or(1);
    shaper
        .response_uptil_now
        .clone_from(&state.response_uptil_now);
    apply_patch(&mut shaper, &state_patch(state));
    shaper
}

fn state_patch(state: &StateInput) -> StatePatch {
    StatePatch {
        sent_first_chunk: state.sent_first_chunk,
        sent_last_chunk: state.sent_last_chunk,
        holding_chunk: (!state.holding_chunk.is_empty()).then(|| state.holding_chunk.clone()),
        received_finish_reason: state.received_finish_reason.clone(),
        intermittent_finish_reason: state.intermittent_finish_reason.clone(),
        tool_call: state.tool_call,
        sent_first_thinking_block: state.sent_first_thinking_block,
        sent_last_thinking_block: state.sent_last_thinking_block,
    }
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

fn run_step(shaper: &mut ChunkShaper, step: &Step, input: &Input) -> StepResult {
    if let Some(result) = creation_step(shaper, step) {
        return result;
    }
    if let Some(result) = decision_step(shaper, step, input) {
        return result;
    }
    if let Some(result) = usage_step(shaper, step) {
        return result;
    }
    if let Some(result) = value_step(shaper, step) {
        return result;
    }
    if let Some(result) = check_step(step) {
        return result;
    }
    Err(failure("InvalidInput", None, "unknown step".to_owned()))
}

fn creation_step(shaper: &mut ChunkShaper, step: &Step) -> Option<StepResult> {
    if let Step::Creator {
        chunk,
        hidden_params,
    } = step
    {
        let response = model_response_creator(shaper, chunk.as_ref(), hidden_params.as_ref())
            .map_err(|error| shape_error(&error));
        return Some(response.and_then(|created| encoded(&created)));
    }
    if let Step::SetModelId { id, chunk } = step {
        let mut response = chunk.clone();
        set_model_id(shaper, id, &mut response);
        return Some(encoded(&response));
    }
    if let Step::CopyProviderFields { original, chunk } = step {
        let mut response = chunk.clone();
        copy_provider_specific_fields(original, &mut response);
        return Some(encoded(&response));
    }
    if let Step::CombineThinking { chunk } = step {
        let mut response = chunk.clone();
        combine_thinking_in_choices(shaper, &mut response);
        return Some(encoded(&response));
    }
    if let Step::ConvertGeneric { chunk } = step {
        let response = convert_generic_chunk(chunk, &SystemStreamIdentity);
        return Some(encoded(&response));
    }
    None
}

fn decision_step(shaper: &mut ChunkShaper, step: &Step, input: &Input) -> Option<StepResult> {
    if matches!(step, Step::FinishReason) {
        let response = finish_reason_handler(shaper).map_err(|error| shape_error(&error));
        return Some(response.and_then(|finished| encoded(&finished)));
    }
    if let Step::Returns {
        completion,
        chunk,
        response,
    } = step
    {
        let mut processed = completion.clone();
        let limit = input.repeated_chunk_limit;
        let sent =
            return_processed_chunk_logic(shaper, &mut processed, chunk.clone(), response, limit)
                .map_err(|error| shape_error(&error));
        return Some(sent.and_then(|processed_chunk| encoded(&processed_chunk)));
    }
    if matches!(step, Step::Repetition) {
        let repeated = raise_on_model_repetition(shaper, input.repeated_chunk_limit)
            .map_err(|error| shape_error(&error));
        return Some(repeated.map(|()| Value::Null));
    }
    if let Step::RecordUsage { chunk } = step {
        record_usage_only_chunk(shaper, chunk.clone());
        return Some(Ok(Value::Null));
    }
    if let Step::SetState { patch } = step {
        apply_patch(shaper, patch);
        return Some(Ok(Value::Null));
    }
    if let Step::PropagateCost { chunk } = step {
        let mut response = chunk.clone();
        let providers = &input.usage_cost_providers;
        propagate_usage_cost(&mut response, shaper.provider.as_deref(), providers);
        return Some(encoded(&response));
    }
    None
}

fn usage_step(shaper: &ChunkShaper, step: &Step) -> Option<StepResult> {
    if matches!(step, Step::TotalUsage) {
        return Some(encoded(&calculate_total_usage(&shaper.chunks)));
    }
    if let Step::BackfillUsage { usage } = step {
        let mut counted = usage.clone();
        backfill_cache_usage(&mut counted);
        return Some(encoded(&counted));
    }
    if let Step::ResolveCost { cost } = step {
        let resolved = resolve_provider_reported_cost(cost).and_then(decimal_json);
        return Some(Ok(resolved.unwrap_or(Value::Null)));
    }
    if let Step::CacheTokenCount { details, keys } = step {
        let names: Vec<&str> = keys.iter().map(String::as_str).collect();
        return Some(Ok(json!(cache_token_count(details.as_ref(), &names))));
    }
    if let Step::CoerceDetails { details } = step {
        return Some(encoded(&coerce_token_details(Some(details))));
    }
    None
}

fn value_step(shaper: &mut ChunkShaper, step: &Step) -> Option<StepResult> {
    if let Step::IsChunkNonEmpty {
        completion,
        chunk,
        response,
    } = step
    {
        let empty = is_chunk_non_empty(shaper, completion, chunk, response);
        return Some(Ok(json!(empty)));
    }
    if let Step::SpecialTokens {
        chunk,
        finish_reason,
    } = step
    {
        let (hold, text) = check_special_tokens(shaper, chunk, finish_reason.as_deref());
        return Some(Ok(json!({"hold": hold, "text": text})));
    }
    if let Step::StripSse { chunk } = step {
        return Some(Ok(json!(strip_sse_data_from_chunk(chunk.as_deref()))));
    }
    if let Step::GenericKnown { chunk } = step {
        return Some(Ok(json!(generic_chunk_has_known_fields(chunk))));
    }
    if let Step::MappedFinishReason { raw } = step {
        return Some(Ok(json!(mapped_finish_reason(raw))));
    }
    None
}

fn check_step(step: &Step) -> Option<StepResult> {
    if let Step::ProviderResponseModel { chunk } = step {
        return Some(Ok(json!(provider_response_model(chunk))));
    }
    if let Step::ProviderHiddenParams {
        chunk,
        provider_response_model,
    } = step
    {
        let params = provider_hidden_params(chunk, provider_response_model.as_deref());
        return Some(params.map_or(Ok(Value::Null), |hidden| encoded(&hidden)));
    }
    if let Step::StampServiceTier { chunk, complete } = step {
        let mut response = chunk.clone();
        stamp_served_service_tier(&mut response, complete);
        return Some(encoded(&response));
    }
    if let Step::CheckStreamUsage { stream_options } = step {
        return Some(Ok(json!(check_send_stream_usage(stream_options.as_ref()))));
    }
    if let Step::CheckFunctionCall { optional_params } = step {
        return Some(Ok(json!(check_is_function_call(Some(optional_params)))));
    }
    None
}
