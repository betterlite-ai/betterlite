use std::str::FromStr as _;

use betterlite_streaming::stream_chunk_shaping::ChunkShaper;
use betterlite_streaming::stream_chunk_shaping::creation::{
    SystemStreamIdentity, copy_provider_specific_fields, model_response_creator,
    provider_hidden_params as compute_hidden_params, provider_response_model as chunk_model,
    set_model_id as adopt_model_id, stamp_served_service_tier,
};
use betterlite_streaming::stream_chunk_shaping::decision::{
    combine_thinking_in_choices, return_processed_chunk_logic,
};
use betterlite_streaming::stream_chunk_shaping::error::StreamShapeError;
use betterlite_streaming::stream_chunk_shaping::generic::convert_generic_chunk;
use betterlite_streaming::stream_chunk_shaping::tokens::{
    check_send_stream_usage, check_special_tokens, finish_reason_handler, mapped_finish_reason,
    raise_on_model_repetition,
};
use betterlite_streaming::stream_chunk_shaping::usage_totals::{
    backfill_cache_usage, cache_token_count as count_cache_tokens, coerce_token_details,
    propagate_usage_cost, record_usage_only_chunk, resolve_provider_reported_cost,
};
use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::{Number, Value, json};

use super::super::{Input, Step};
use crate::helpers::HelperError;

pub(super) fn creator(shaper: &mut ChunkShaper, step: &Step) -> Result<Value, Box<HelperError>> {
    let Step::Creator {
        chunk,
        hidden_params,
    } = step
    else {
        return Ok(Value::Null);
    };
    let response = model_response_creator(shaper, chunk.as_ref(), hidden_params.as_ref())
        .map_err(|error| shape_error(&error))?;
    encoded(&response)
}

pub(super) fn set_model_id(
    shaper: &mut ChunkShaper,
    step: &Step,
) -> Result<Value, Box<HelperError>> {
    let Step::SetModelId { id, chunk } = step else {
        return Ok(Value::Null);
    };
    let mut response = chunk.clone();
    adopt_model_id(shaper, id, &mut response);
    encoded(&response)
}

pub(super) fn copy_provider_fields(step: &Step) -> Result<Value, Box<HelperError>> {
    let Step::CopyProviderFields { original, chunk } = step else {
        return Ok(Value::Null);
    };
    let mut response = chunk.clone();
    copy_provider_specific_fields(original, &mut response);
    encoded(&response)
}

pub(super) fn returns(
    shaper: &mut ChunkShaper,
    step: &Step,
    input: &Input,
) -> Result<Value, Box<HelperError>> {
    let Step::Returns {
        completion,
        chunk,
        response,
    } = step
    else {
        return Ok(Value::Null);
    };
    let mut processed_completion = completion.clone();
    let sent = return_processed_chunk_logic(
        shaper,
        &mut processed_completion,
        chunk.clone(),
        response,
        input.repeated_chunk_limit,
    )
    .map_err(|error| shape_error(&error))?;
    encoded(&sent)
}

pub(super) fn combine_thinking(
    shaper: &mut ChunkShaper,
    step: &Step,
) -> Result<Value, Box<HelperError>> {
    let Step::CombineThinking { chunk } = step else {
        return Ok(Value::Null);
    };
    let mut response = chunk.clone();
    combine_thinking_in_choices(shaper, &mut response);
    encoded(&response)
}

pub(super) fn special_tokens(shaper: &mut ChunkShaper, step: &Step) -> Value {
    let Step::SpecialTokens {
        chunk,
        finish_reason,
    } = step
    else {
        return Value::Null;
    };
    let (hold, text) = check_special_tokens(shaper, chunk, finish_reason.as_deref());
    json!({"hold": hold, "text": text})
}

pub(super) fn repetition(
    shaper: &mut ChunkShaper,
    input: &Input,
) -> Result<Value, Box<HelperError>> {
    raise_on_model_repetition(shaper, input.repeated_chunk_limit)
        .map_err(|error| shape_error(&error))?;
    Ok(Value::Null)
}

pub(super) fn finish_reason(shaper: &mut ChunkShaper) -> Result<Value, Box<HelperError>> {
    serde_json::to_value(&finish_reason_handler(shaper).map_err(|error| shape_error(&error))?)
        .map_err(|error| serialization_error(&error))
}

pub(super) fn record_usage(shaper: &mut ChunkShaper, step: &Step) -> Value {
    let Step::RecordUsage { chunk } = step else {
        return Value::Null;
    };
    record_usage_only_chunk(shaper, chunk.clone());
    Value::Null
}

pub(super) fn convert_generic(step: &Step) -> Result<Value, Box<HelperError>> {
    let Step::ConvertGeneric { chunk } = step else {
        return Ok(Value::Null);
    };
    let response = convert_generic_chunk(chunk, &SystemStreamIdentity);
    encoded(&response)
}

pub(super) fn backfill(step: &Step) -> Result<Value, Box<HelperError>> {
    let Step::BackfillUsage { usage } = step else {
        return Ok(Value::Null);
    };
    let mut counted = usage.clone();
    backfill_cache_usage(&mut counted);
    encoded(&counted)
}

pub(super) fn resolved_cost(step: &Step) -> Value {
    let Step::ResolveCost { cost } = step else {
        return Value::Null;
    };
    resolve_provider_reported_cost(cost)
        .and_then(decimal_json)
        .unwrap_or(Value::Null)
}

pub(super) fn propagate_cost(
    shaper: &ChunkShaper,
    step: &Step,
    input: &Input,
) -> Result<Value, Box<HelperError>> {
    let Step::PropagateCost { chunk } = step else {
        return Ok(Value::Null);
    };
    let mut response = chunk.clone();
    propagate_usage_cost(
        &mut response,
        shaper.provider.as_deref(),
        &input.usage_cost_providers,
    );
    encoded(&response)
}

pub(super) fn cache_token_count(step: &Step) -> Value {
    let Step::CacheTokenCount { details, keys } = step else {
        return Value::Null;
    };
    let names: Vec<&str> = keys.iter().map(String::as_str).collect();
    json!(count_cache_tokens(details.as_ref(), &names))
}

pub(super) fn coerce_details(step: &Step) -> Result<Value, Box<HelperError>> {
    let Step::CoerceDetails { details } = step else {
        return Ok(Value::Null);
    };
    let coerced = coerce_token_details(Some(details));
    encoded(&coerced)
}

pub(super) fn provider_response_model(step: &Step) -> Value {
    let Step::ProviderResponseModel { chunk } = step else {
        return Value::Null;
    };
    json!(chunk_model(chunk))
}

pub(super) fn provider_hidden_params(step: &Step) -> Result<Value, Box<HelperError>> {
    let Step::ProviderHiddenParams {
        chunk,
        provider_response_model,
    } = step
    else {
        return Ok(Value::Null);
    };
    compute_hidden_params(chunk, provider_response_model.as_deref())
        .map_or(Ok(Value::Null), |params| encoded(&params))
}

pub(super) fn stamp_service_tier(step: &Step) -> Result<Value, Box<HelperError>> {
    let Step::StampServiceTier { chunk, complete } = step else {
        return Ok(Value::Null);
    };
    let mut response = chunk.clone();
    stamp_served_service_tier(&mut response, complete);
    encoded(&response)
}

pub(super) fn check_stream_usage(step: &Step) -> Value {
    let Step::CheckStreamUsage { stream_options } = step else {
        return Value::Null;
    };
    json!(check_send_stream_usage(stream_options.as_ref()))
}

pub(super) fn mapped_finish_reason_value(step: &Step) -> Value {
    let Step::MappedFinishReason { raw } = step else {
        return Value::Null;
    };
    json!(mapped_finish_reason(raw))
}

pub(super) fn encoded<T>(value: &T) -> Result<Value, Box<HelperError>>
where
    T: Serialize,
{
    serde_json::to_value(value).map_err(|error| {
        Box::new(HelperError {
            class: "SerializationError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })
}

fn decimal_json(cost: Decimal) -> Option<Value> {
    Number::from_str(&cost.normalize().to_string())
        .ok()
        .map(Value::Number)
}

fn serialization_error(error: &serde_json::Error) -> Box<HelperError> {
    Box::new(HelperError {
        class: "SerializationError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn shape_error(error: &StreamShapeError) -> Box<HelperError> {
    let (class, status) = match error {
        StreamShapeError::Repetition { .. } => ("InternalServerError", Some(500_u16)),
        StreamShapeError::Arguments { .. } => ("ValidationError", None),
    };
    Box::new(HelperError {
        class: class.to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
