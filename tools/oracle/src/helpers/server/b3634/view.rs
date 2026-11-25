use std::str::FromStr as _;

use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_server::pass_through_logging::response_costing::ResponseCostRequest;
use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::{Value, json};

pub(super) const BATCH_RESPONSE_ID: &str = "b3634-batch-id";

// The completion_cost arguments the payload step decides: the costing model, provider, pricing flags and speed.
#[derive(Debug, Serialize)]
pub(super) struct PricedCall {
    model: String,
    custom_llm_provider: Option<String>,
    custom_pricing: Option<bool>,
    router_model_id: Option<String>,
    speed: Option<String>,
}

impl PricedCall {
    pub(super) fn of(request: &ResponseCostRequest<'_>) -> Self {
        let response = serde_json::to_value(request.response).unwrap_or(Value::Null);
        Self {
            model: request.model.to_owned(),
            custom_llm_provider: request.custom_llm_provider.map(str::to_owned),
            custom_pricing: request.custom_pricing,
            router_model_id: request.router_model_id.map(str::to_owned),
            speed: response
                .pointer("/usage/speed")
                .and_then(Value::as_str)
                .map(str::to_owned),
        }
    }
}

// The stream build prices its own response first, so the payload step's price is the last one recorded.
pub(super) fn outcome_view(
    outcome: &HandlerOutcome,
    logging_obj: &StreamLoggingObject,
    calls: &[PricedCall],
) -> HelperResult {
    let result = match &outcome.result {
        Some(response) => serde_json::to_value(response.clone().into_json()).or_value_error()?,
        None => Value::Null,
    };
    let cost = match calls.last() {
        Some(call) if !result.is_null() => serde_json::to_value(call).or_value_error()?,
        Some(_) | None => Value::Null,
    };
    let kwargs = serde_json::to_value(&outcome.kwargs.entries).or_value_error()?;
    let details = logging_obj
        .read(&mut |state| serde_json::to_value(&state.model_call_details))
        .ok_or("the call log left its slot")
        .or_value_error()?
        .or_value_error()?;
    Ok(json!({
        "result": result_view(&result),
        "kwargs": kwargs_view(&kwargs),
        "details": details_view(&details),
        "cost": cost,
    }))
}

fn result_view(body: &Value) -> Value {
    if body.is_null() {
        return Value::Null;
    }
    let choices: Vec<Value> = body
        .get("choices")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .map(|row| {
                    json!({
                        "finish_reason": row.get("finish_reason"),
                        "role": row.pointer("/message/role"),
                        "content": row.pointer("/message/content"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    json!({
        "id": body.get("id"),
        "object": body.get("object"),
        "model": body.get("model"),
        "choices": choices,
        "usage": usage_view(body.get("usage")),
    })
}

fn usage_view(usage: Option<&Value>) -> Value {
    let Some(fields) = usage.filter(|value| value.is_object()) else {
        return Value::Null;
    };
    json!({
        "prompt_tokens": fields.get("prompt_tokens"),
        "completion_tokens": fields.get("completion_tokens"),
        "total_tokens": fields.get("total_tokens"),
        "speed": fields.get("speed"),
    })
}

fn kwargs_view(kwargs: &Value) -> Value {
    json!({
        "response_cost": cost_text(kwargs.get("response_cost")),
        "model": kwargs.get("model"),
        "litellm_params": kwargs.get("litellm_params"),
        "batch_id": kwargs.get("batch_id"),
        "unified_object_id": kwargs.get("unified_object_id"),
        "batch_job_state": kwargs.get("batch_job_state"),
    })
}

fn details_view(details: &Value) -> Value {
    let complete = details
        .get("complete_streaming_response")
        .filter(|value| value.is_object())
        .map(|stored| json!({ "id": stored.get("id"), "model": stored.get("model") }));
    json!({
        "model": details.get("model"),
        "response_cost": cost_text(details.get("response_cost")),
        "custom_llm_provider": details.get("custom_llm_provider"),
        "batch_id": details.get("batch_id"),
        "complete_streaming_response": complete,
    })
}

// A float cost and a decimal cost compare as their normalized decimal text.
fn cost_text(cost: Option<&Value>) -> Value {
    let Some(Value::Number(number)) = cost else {
        return Value::Null;
    };
    Decimal::from_str(&number.to_string())
        .map_or(Value::Null, |parsed| json!(parsed.normalize().to_string()))
}
