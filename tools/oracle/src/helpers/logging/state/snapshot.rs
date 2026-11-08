use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::correlation::CorrelationContext;
use serde_json::{Map, Value};

pub(super) fn snapshot(state: &LoggingState, context: &CorrelationContext) -> Value {
    let mut fields = Map::new();
    insert_identity(&mut fields, state);
    insert_request(&mut fields, state);
    insert_settings(&mut fields, state);
    insert_correlation(&mut fields, state, context);
    Value::Object(fields)
}

fn insert_identity(fields: &mut Map<String, Value>, state: &LoggingState) {
    fields.insert("model".to_owned(), serde_json::json!(&state.model));
    fields.insert("messages".to_owned(), serde_json::json!(&state.messages));
    fields.insert("stream".to_owned(), serde_json::json!(state.stream));
    fields.insert("call_type".to_owned(), serde_json::json!(&state.call_type));
    fields.insert(
        "start_time_micros".to_owned(),
        serde_json::json!(state.start_time.timestamp_micros()),
    );
    fields.insert(
        "litellm_call_id".to_owned(),
        serde_json::json!(&state.litellm_call_id),
    );
    fields.insert(
        "function_id".to_owned(),
        serde_json::json!(&state.function_id),
    );
}

fn insert_request(fields: &mut Map<String, Value>, state: &LoggingState) {
    fields.insert(
        "litellm_trace_id".to_owned(),
        serde_json::json!(&state.litellm_trace_id),
    );
    fields.insert(
        "litellm_session_id".to_owned(),
        serde_json::json!(&state.litellm_session_id),
    );
    fields.insert(
        "litellm_params".to_owned(),
        serde_json::json!(&state.litellm_params),
    );
    fields.insert(
        "optional_params".to_owned(),
        serde_json::json!(&state.optional_params),
    );
    fields.insert("user".to_owned(), serde_json::json!(&state.user));
    fields.insert(
        "litellm_request_debug".to_owned(),
        serde_json::json!(state.litellm_request_debug),
    );
    fields.insert(
        "stream_options".to_owned(),
        serde_json::json!(&state.stream_options),
    );
}

fn insert_settings(fields: &mut Map<String, Value>, state: &LoggingState) {
    fields.insert(
        "custom_pricing".to_owned(),
        serde_json::json!(state.cost.custom_pricing),
    );
    fields.insert(
        "zero_cost_warned".to_owned(),
        serde_json::json!(state.cost.zero_cost_warned),
    );
    fields.insert(
        "custom_llm_provider".to_owned(),
        serde_json::json!(&state.custom_llm_provider),
    );
    fields.insert(
        "completion_start_time_micros".to_owned(),
        serde_json::json!(
            state
                .completion_start_time
                .map(|time| time.timestamp_micros())
        ),
    );
    fields.insert(
        "response_timing_metrics".to_owned(),
        serde_json::json!(&state.response_timing_metrics),
    );
    fields.insert(
        "model_call_details".to_owned(),
        serde_json::json!(&state.model_call_details),
    );
    fields.insert(
        "callback_dynamic_params".to_owned(),
        serde_json::json!(&state.callback_dynamic_params),
    );
    fields.insert(
        "trusted_callback_vars".to_owned(),
        serde_json::json!(&state.trusted_callback_vars),
    );
    fields.insert(
        "standard_built_in_tools_params".to_owned(),
        serde_json::json!(&state.standard_built_in_tools_params),
    );
    fields.insert(
        "request_logging".to_owned(),
        serde_json::json!(state.request_logging),
    );
    fields.insert(
        "dynamic_callbacks".to_owned(),
        serde_json::json!(&state.dynamic_callbacks),
    );
    fields.insert(
        "truncated_messages_for_logging".to_owned(),
        serde_json::json!(&state.truncated_messages_for_logging),
    );
    fields.insert(
        "cost_breakdown".to_owned(),
        serde_json::json!(&state.cost_breakdown),
    );
}

fn insert_correlation(
    fields: &mut Map<String, Value>,
    state: &LoggingState,
    context: &CorrelationContext,
) {
    fields.insert(
        "correlation".to_owned(),
        serde_json::json!({
            "pre_call": &state.correlation.pre_call,
            "own": &state.correlation.own,
        }),
    );
    fields.insert("context".to_owned(), serde_json::json!(context));
    fields.insert(
        "async_success_scheduled".to_owned(),
        serde_json::json!(state.deferred.async_success_scheduled),
    );
}

pub(super) fn context(context: &CorrelationContext) -> Value {
    serde_json::json!({
        "trace_id": &context.trace_id,
        "session_id": &context.session_id,
    })
}
