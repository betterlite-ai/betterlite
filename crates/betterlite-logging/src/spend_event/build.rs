use std::str::FromStr as _;

use betterlite_models::decisions::Json;
use rust_decimal::Decimal;
use serde_json::Number;

use crate::spend_event::CACHE_OFF_KEY;
use crate::spend_event::error::SpendEventBuildError;
use crate::spend_event::event::{EventVersion, LitellmParams, Mapping, SpendEvent};
use crate::spend_event::input::{BuildInput, CachePresence, SuccessKwargs, TypedAuths};
use crate::spend_event::reservation::{choose_reservation, event_bucket};
use crate::spend_event::response::compact_response;
use crate::spend_event::tools::request_tool_names;
use crate::spend_event::usage::usage_counts;

// Python writes every float with a point or an exponent, so a whole cost keeps one decimal digit
fn float_scale(mut cost: Decimal) -> Decimal {
    if cost.scale() == 0 {
        cost.rescale(1);
    }
    cost
}

fn cost_from_number(number: &Number) -> Result<Decimal, SpendEventBuildError> {
    let text = number.to_string();
    Decimal::from_str(&text)
        .or_else(|_| Decimal::from_scientific(&text))
        .map(float_scale)
        .map_err(|source| SpendEventBuildError::ResponseCost {
            value: text,
            source,
        })
}

// A present standard logging object, even an empty one, blocks the kwargs fallback, and booleans count as numbers
fn event_cost(kwargs: &SuccessKwargs) -> Result<Option<Decimal>, SpendEventBuildError> {
    let Some(object) = &kwargs.standard_logging_object else {
        return Ok(kwargs.response_cost.map(float_scale));
    };
    match object.get("response_cost") {
        Some(Json::Bool(flag)) => Ok(Some(float_scale(Decimal::from(u8::from(*flag))))),
        Some(Json::Number(number)) => cost_from_number(number).map(Some),
        Some(Json::Null | Json::Text(_) | Json::List(_) | Json::Object(_)) | None => Ok(None),
    }
}

// The key the caching layer already stored, or the literal Cache OFF when the caller has no cache
#[must_use]
pub fn preset_spend_log_cache_key(params: &LitellmParams, cache: CachePresence) -> Option<String> {
    match cache {
        CachePresence::Off => Some(CACHE_OFF_KEY.to_owned()),
        CachePresence::On => params.preset_cache_key.clone(),
    }
}

fn project_params(
    params: LitellmParams,
    typed: &TypedAuths,
    cache: CachePresence,
    store_bodies: bool,
) -> LitellmParams {
    let reservation = choose_reservation(&params, typed);
    let preset_cache_key = preset_spend_log_cache_key(&params, cache);
    LitellmParams {
        metadata: event_bucket(params.metadata.as_ref(), reservation.as_ref()),
        litellm_metadata: event_bucket(params.litellm_metadata.as_ref(), reservation.as_ref()),
        proxy_server_request: if store_bodies {
            params.proxy_server_request
        } else {
            None
        },
        preset_cache_key,
        api_base: params.api_base,
        custom_llm_provider: params.custom_llm_provider,
        litellm_call_id: params.litellm_call_id,
        user_api_key_end_user_id: params.user_api_key_end_user_id,
    }
}

fn project_standard_logging(object: Option<Mapping>, store_bodies: bool) -> Option<Mapping> {
    let mut kept = object?;
    kept.shift_remove("model_parameters");
    if !store_bodies {
        kept.shift_remove("messages");
        kept.shift_remove("response");
    }
    Some(kept)
}

fn parse_kwargs(raw: &Mapping) -> Result<SuccessKwargs, SpendEventBuildError> {
    let text =
        serde_json::to_string(raw).map_err(|source| SpendEventBuildError::Encode { source })?;
    serde_json::from_str(&text).map_err(|source| SpendEventBuildError::Kwargs { source })
}

fn assemble(
    kwargs: SuccessKwargs,
    input: &BuildInput<'_>,
    response_cost: Option<Decimal>,
    tool_names: Vec<String>,
) -> SpendEvent {
    SpendEvent {
        version: EventVersion::One,
        litellm_call_id: kwargs.litellm_call_id,
        call_type: kwargs.call_type,
        model: kwargs.model,
        custom_llm_provider: kwargs.custom_llm_provider,
        stream: kwargs.stream,
        complete_streaming_response: !kwargs.complete_streaming_response.is_missing(),
        cache_hit: kwargs.cache_hit,
        response_cost,
        start_time: input.start_time.clone(),
        end_time: input.end_time.clone(),
        completion_start_time: kwargs.completion_start_time,
        agent_id: kwargs.agent_id,
        litellm_trace_id: kwargs.litellm_trace_id,
        litellm_params: project_params(
            kwargs.litellm_params,
            input.typed_auth,
            input.cache,
            input.store_bodies,
        ),
        standard_logging_object: project_standard_logging(
            kwargs.standard_logging_object,
            input.store_bodies,
        ),
        dynamic_params: kwargs.dynamic_params,
        response: compact_response(input.response),
        combined_usage: kwargs.combined_usage_object,
        realtime_tools: kwargs.realtime_tools,
        realtime_tool_calls: kwargs.realtime_tool_calls,
        request_tool_names: tool_names,
    }
}

// Validates the selected callback kwargs and writes the event once, as one newline-terminated JSON line
pub fn build_spend_event(input: &BuildInput<'_>) -> Result<Vec<u8>, SpendEventBuildError> {
    let kwargs = parse_kwargs(input.kwargs)?;
    let tool_names = request_tool_names(&kwargs);
    let response_cost = event_cost(&kwargs)?;
    if let Some(usage) = &kwargs.combined_usage_object {
        usage_counts(usage).map_err(|source| SpendEventBuildError::Usage { source })?;
    }
    let event = assemble(kwargs, input, response_cost, tool_names);
    let mut line =
        serde_json::to_vec(&event).map_err(|source| SpendEventBuildError::Encode { source })?;
    line.push(b'\n');
    Ok(line)
}
