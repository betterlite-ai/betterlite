use betterlite_streaming::stream_chunk_shaping::{
    ChunkShaper,
    creation::{
        model_response_creator, provider_hidden_params, provider_response_model, set_model_id,
        stamp_served_service_tier, system_instant,
    },
    decision::{combine_thinking_in_choices, is_chunk_non_empty, return_processed_chunk_logic},
    extras::{convert_generic_chunk, generic_chunk_has_known_fields, strip_sse_data_from_chunk},
    records::{ChunkResponseObject, GenericChunk, JsonObject, StreamChunk, StreamDelta, Usage},
    tokens::{check_special_tokens, finish_reason_handler, raise_on_model_repetition},
    usage_totals::{backfill_cache_usage, calculate_total_usage, propagate_usage_cost},
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::helpers::HelperError;
use ops::{encoded, failure, shape_error};

mod ops;

type StepResult = Result<Value, Box<HelperError>>;

pub(super) fn run(input: &Value) -> StepResult {
    let mut shaper = build_shaper(input)?;
    let limit: usize = field(input, "repeated_chunk_limit")?.unwrap_or(0);
    let providers: Vec<String> = field(input, "usage_cost_providers")?.unwrap_or_default();
    let steps: Vec<Value> = field(input, "steps")?.unwrap_or_default();
    let mut outputs = Vec::with_capacity(steps.len());
    for step in &steps {
        let result = apply(&mut shaper, step, limit, &providers)?;
        outputs.push(json!({"result": result}));
    }
    Ok(Value::Array(outputs))
}

fn field<T>(value: &Value, key: &str) -> Result<Option<T>, Box<HelperError>>
where
    T: DeserializeOwned,
{
    let Some(raw) = value.get(key).filter(|raw| !raw.is_null()) else {
        return Ok(None);
    };
    serde_json::from_value(raw.clone())
        .map(Some)
        .map_err(|error| failure("InvalidInput", None, error.to_string()))
}

fn build_shaper(input: &Value) -> Result<ChunkShaper, Box<HelperError>> {
    let state = input.get("state").cloned().unwrap_or_default();
    let mut shaper = ChunkShaper::new(
        field::<String>(&state, "model")?.unwrap_or_default(),
        field(&state, "provider")?,
        field(&state, "merge_reasoning_in_choices")?.unwrap_or(false),
        field::<JsonObject>(&state, "base_hidden_params")?.unwrap_or_default(),
    );
    shaper.response_id = field(&state, "response_id")?;
    shaper.created = field(&state, "created")?;
    shaper.system_fingerprint = field(&state, "system_fingerprint")?;
    shaper.chunks = field(&state, "chunks")?.unwrap_or_default();
    shaper.tool_call = field(&state, "tool_call")?.unwrap_or(false);
    Ok(shaper)
}

fn apply(shaper: &mut ChunkShaper, step: &Value, limit: usize, providers: &[String]) -> StepResult {
    match field::<String>(step, "op")?.as_deref() {
        Some("creator") => creator(shaper, step),
        Some("set_model_id") => set_model_id_step(shaper, step),
        Some("combine_thinking") => combine_thinking(shaper, step),
        Some("convert_generic") => encoded(&convert_generic_chunk(
            &field::<GenericChunk>(step, "chunk")?.unwrap_or_default(),
            system_instant,
            Uuid::new_v4,
        )),
        Some("is_chunk_non_empty") => chunk_non_empty(shaper, step),
        Some("returns") => returns(shaper, step, limit),
        Some("repetition") => repetition(shaper, limit),
        Some("finish_reason") => encoded(&finish_reason_handler(shaper)),
        Some("special_tokens") => special_tokens(shaper, step),
        Some("strip_sse") => Ok(json!(strip_sse_data_from_chunk(
            field::<String>(step, "chunk")?.as_deref()
        ))),
        Some("generic_known") => Ok(json!(generic_chunk_has_known_fields(
            &field::<JsonObject>(step, "chunk")?.unwrap_or_default(),
        ))),
        Some("total_usage") => encoded(&calculate_total_usage(&shaper.chunks)),
        Some("backfill_usage") => backfill_usage(step),
        Some("propagate_cost") => propagate_cost(shaper, step, providers),
        Some("provider_response_model") => Ok(json!(provider_response_model(
            &field::<StreamChunk>(step, "chunk")?.unwrap_or_default(),
        ))),
        Some("provider_hidden_params") => hidden_params(step),
        Some("stamp_service_tier") => service_tier(step),
        Some("set_state") => set_state(shaper, step),
        other => Err(failure(
            "InvalidInput",
            None,
            format!("unsupported operation {other:?}"),
        )),
    }
}

fn creator(shaper: &mut ChunkShaper, step: &Value) -> StepResult {
    let chunk: Option<JsonObject> = field(step, "chunk")?;
    let hidden: Option<JsonObject> = field(step, "hidden_params")?;
    let created = model_response_creator(shaper, chunk.as_ref(), hidden.as_ref())
        .map_err(|error| shape_error(&error))?;
    encoded(&created)
}

fn set_model_id_step(shaper: &mut ChunkShaper, step: &Value) -> StepResult {
    let mut response: StreamChunk = field(step, "chunk")?.unwrap_or_default();
    let id: String = field(step, "id")?.unwrap_or_default();
    set_model_id(shaper, &id, &mut response);
    encoded(&response)
}

fn combine_thinking(shaper: &mut ChunkShaper, step: &Value) -> StepResult {
    let mut response: StreamChunk = field(step, "chunk")?.unwrap_or_default();
    combine_thinking_in_choices(shaper, &mut response);
    encoded(&response)
}

fn chunk_non_empty(shaper: &ChunkShaper, step: &Value) -> StepResult {
    let completion: StreamDelta = field(step, "completion")?.unwrap_or_default();
    let chunk: StreamChunk = field(step, "chunk")?.unwrap_or_default();
    let response: ChunkResponseObject = field(step, "response")?.unwrap_or_default();
    Ok(json!(is_chunk_non_empty(
        shaper,
        &completion,
        &chunk,
        &response
    )))
}

fn returns(shaper: &mut ChunkShaper, step: &Value, limit: usize) -> StepResult {
    let mut completion: StreamDelta = field(step, "completion")?.unwrap_or_default();
    let chunk: StreamChunk = field(step, "chunk")?.unwrap_or_default();
    let response: ChunkResponseObject = field(step, "response")?.unwrap_or_default();
    let sent = return_processed_chunk_logic(shaper, &mut completion, chunk, &response, limit)
        .map_err(|error| shape_error(&error))?;
    encoded(&sent)
}

fn repetition(shaper: &mut ChunkShaper, limit: usize) -> StepResult {
    raise_on_model_repetition(shaper, limit).map_err(|error| shape_error(&error))?;
    Ok(Value::Null)
}

fn special_tokens(shaper: &mut ChunkShaper, step: &Value) -> StepResult {
    let chunk: String = field(step, "chunk")?.unwrap_or_default();
    let reason: Option<String> = field(step, "finish_reason")?;
    let (hold, text) = check_special_tokens(shaper, &chunk, reason.as_deref());
    Ok(json!({"hold": hold, "text": text}))
}

fn backfill_usage(step: &Value) -> StepResult {
    let mut usage: Usage = field(step, "usage")?.unwrap_or_default();
    backfill_cache_usage(&mut usage);
    encoded(&usage)
}

fn propagate_cost(shaper: &ChunkShaper, step: &Value, providers: &[String]) -> StepResult {
    let mut response: StreamChunk = field(step, "chunk")?.unwrap_or_default();
    propagate_usage_cost(&mut response, shaper.provider.as_deref(), providers);
    encoded(&response)
}

fn hidden_params(step: &Value) -> StepResult {
    let chunk: StreamChunk = field(step, "chunk")?.unwrap_or_default();
    let model: Option<String> = field(step, "provider_response_model")?;
    encoded(&provider_hidden_params(&chunk, model.as_deref()))
}

fn service_tier(step: &Value) -> StepResult {
    let mut response: StreamChunk = field(step, "chunk")?.unwrap_or_default();
    let complete: StreamChunk = field(step, "complete")?.unwrap_or_default();
    stamp_served_service_tier(&mut response, &complete);
    encoded(&response)
}

fn set_state(shaper: &mut ChunkShaper, step: &Value) -> StepResult {
    let empty = Value::Null;
    let patch = step.get("patch").unwrap_or(&empty);
    if let Some(value) = field(patch, "sent_first_chunk")? {
        shaper.progress.sent_first_chunk = value;
    }
    if let Some(value) = field(patch, "sent_last_chunk")? {
        shaper.progress.sent_last_chunk = value;
    }
    if let Some(value) = field::<String>(patch, "holding_chunk")? {
        shaper.holding_chunk = value;
    }
    if let Some(value) = field(patch, "received_finish_reason")? {
        shaper.received_finish_reason = Some(value);
    }
    if let Some(value) = field(patch, "intermittent_finish_reason")? {
        shaper.intermittent_finish_reason = Some(value);
    }
    if let Some(value) = field(patch, "tool_call")? {
        shaper.tool_call = value;
    }
    if let Some(value) = field(patch, "sent_first_thinking_block")? {
        shaper.thinking.sent_first_block = value;
    }
    if let Some(value) = field(patch, "sent_last_thinking_block")? {
        shaper.thinking.sent_last_block = value;
    }
    Ok(Value::Null)
}
