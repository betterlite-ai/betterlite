use std::str::FromStr as _;

use betterlite_core::cost::prepare::TypedResponse;
use betterlite_core::cost::recording::{RecordInput, RecordResponse};
use betterlite_core::cost::usage::{BillingUsage, ResponseKind};
use betterlite_logging::fields::usage::Usage;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use rust_decimal::Decimal;
use serde_json::Number;

use super::StreamCostInjector;
use super::StreamCostRecorder;

pub(super) const RESPONSE_COST_DEBUG: &str = "response_cost_failure_debug_information";

pub(super) fn streamed_cost(
    usage: &Usage,
    model_name: &str,
    service_tier: Option<&str>,
    injector: StreamCostInjector<'_>,
    log: Option<&mut LoggingState>,
) -> Option<Decimal> {
    if let Some(state) = log
        && let Some(cost) = recorded_cost(state, usage, injector.recorder)
    {
        return Some(cost);
    }
    injector
        .calculator
        .completion_cost(usage, model_name, service_tier)
}

// LiteLLM prices the frame through ModelResponse(usage=...), which is B1717's typed chat record.
fn stream_record(usage: &Usage) -> RecordInput<'_> {
    RecordInput {
        response: RecordResponse::Typed {
            response: TypedResponse {
                kind: ResponseKind::Chat,
                model: None,
                usage: BillingUsage::Chat(usage),
                service_tier: None,
                hidden_params: None,
            },
            background: false,
        },
        cache_hit: None,
        litellm_model_name: None,
        router_model_id: None,
    }
}

// The recorder may stamp cost_breakdown and the failure diagnostic, and neither may outlive this attempt.
fn recorded_cost(
    state: &mut LoggingState,
    usage: &Usage,
    recorder: &dyn StreamCostRecorder,
) -> Option<Decimal> {
    let breakdown = state.cost_breakdown.clone();
    let debug = state.model_call_details.get(RESPONSE_COST_DEBUG).cloned();
    let cost = recorder.response_cost(state, stream_record(usage));
    state.cost_breakdown = breakdown;
    restore_debug(state, debug);
    cost
}

fn restore_debug(state: &mut LoggingState, debug: Option<Json>) {
    match debug {
        Some(value) => {
            state
                .model_call_details
                .insert(RESPONSE_COST_DEBUG.to_owned(), value);
        }
        None => drop(state.model_call_details.shift_remove(RESPONSE_COST_DEBUG)),
    }
}

// Python writes a float, so a whole charge keeps one decimal place and reads the same in both.
pub(super) fn cost_value(cost: Decimal) -> Json {
    let mut normalized = cost.normalize();
    if normalized.scale() == 0_u32 {
        normalized.rescale(1_u32);
    }
    match Number::from_str(&normalized.to_string()) {
        Ok(number) => Json::Number(number),
        Err(_error) => Json::Null,
    }
}
