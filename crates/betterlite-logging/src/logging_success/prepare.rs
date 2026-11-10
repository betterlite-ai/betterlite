use std::sync::Arc;

use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};

use super::handler::SuccessHandler;
use super::payload_state::set_standard_logging_object;
use super::recognized::is_recognized_call_type;
use super::types::{SuccessRequest, SuccessResult};
use crate::callback_logger::event::GuardrailRequestSnapshot;
use crate::fields::payload::StandardLoggingPayload;
use crate::logging_state::LoggingState;
use crate::logging_state::events::update_completion_start_time;

pub struct Prepared {
    pub start_time: DateTime<FixedOffset>,
    pub end_time: DateTime<FixedOffset>,
    pub result: SuccessResult,
    pub guardrail_snapshot: Option<Arc<GuardrailRequestSnapshot>>,
}

// LiteLLM's _success_handler_helper_fn: timing, the call-detail markers and the standard payload
pub(super) fn prepare(
    handler: &SuccessHandler<'_>,
    state: &mut LoggingState,
    request: &SuccessRequest,
) -> Prepared {
    let start_time = request.start_time.unwrap_or(state.start_time);
    let end_time = request.end_time.unwrap_or_else(|| handler.clock.now());
    if state.completion_start_time.is_none() {
        update_completion_start_time(state, end_time);
    }
    state.model_call_details.insert(
        "log_event_type".to_owned(),
        Json::Text("successful_api_call".to_owned()),
    );
    state
        .model_call_details
        .insert("end_time".to_owned(), Json::Text(end_time.to_rfc3339()));
    state.model_call_details.insert(
        "cache_hit".to_owned(),
        request.cache_hit.map_or(Json::Null, Json::Bool),
    );
    let result = request.result.clone();
    if request.standard_logging_object.is_none()
        && state.stream != Some(true)
        && recognizes_service_call(state, &result)
    {
        let payload = handler
            .payload_builder
            .build(state, result.response(), start_time, end_time);
        store_payload(state, payload.as_ref());
    } else if let Some(payload) = request.standard_logging_object.as_ref() {
        store_payload(state, Some(payload));
    } else {
        state
            .model_call_details
            .entry("response_cost".to_owned())
            .or_insert(Json::Null);
    }
    Prepared {
        start_time,
        end_time,
        result,
        guardrail_snapshot: request.guardrail_snapshot.clone(),
    }
}

pub(super) fn store_payload(state: &mut LoggingState, payload: Option<&StandardLoggingPayload>) {
    if let Err(error) = set_standard_logging_object(state, payload) {
        tracing::warn!(%error, "the standard logging payload could not be stored");
    }
}

// LiteLLM builds nothing for a missing result, and also logs a plain dict or list no call type recognizes
fn recognizes_service_call(state: &LoggingState, result: &SuccessResult) -> bool {
    if matches!(result, SuccessResult::Other(Json::Null)) {
        return false;
    }
    is_recognized_call_type(result, &state.call_type)
        || matches!(
            result,
            SuccessResult::Other(Json::Object(_) | Json::List(_))
        )
}
