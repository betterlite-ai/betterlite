use betterlite_models::decisions::Json;

use crate::logging_state::LoggingState;
use crate::logging_state::events::LoggingEvent;

const LITELLM_PARAMS: &str = "litellm_params";
const METADATA: &str = "metadata";
const OTEL_INTERNAL: &str = "_otel_internal";
const SPANS_LOGGED: &str = "spans_logged";

pub fn reset_success_emission_dedupe(state: &mut LoggingState) {
    state
        .model_call_details
        .shift_remove(LoggingEvent::AsyncSuccess.key());
    let Some(Json::Object(litellm_params)) = state.model_call_details.get_mut(LITELLM_PARAMS)
    else {
        return;
    };
    let Some(Json::Object(metadata)) = litellm_params.get_mut(METADATA) else {
        return;
    };
    let Some(Json::Object(otel_internal)) = metadata.get_mut(OTEL_INTERNAL) else {
        return;
    };
    let Some(Json::Object(spans_logged)) = otel_internal.get_mut(SPANS_LOGGED) else {
        return;
    };
    spans_logged.retain(|scope, _logged| !is_success_scope(scope));
}

// a JSON object key is text, so a LiteLLM tuple key is stored in its JSON array form
fn is_success_scope(scope: &str) -> bool {
    let Ok(parts) = serde_json::from_str::<Vec<Json>>(scope) else {
        return false;
    };
    matches!(parts.last(), Some(Json::Text(last)) if last == "success")
}
