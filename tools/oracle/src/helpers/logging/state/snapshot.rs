use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::correlation::CorrelationContext;
use serde_json::Value;

pub(super) fn snapshot(state: &LoggingState, context: &CorrelationContext) -> Value {
    serde_json::json!({
        "model": &state.model,
        "messages": &state.messages,
        "stream": state.stream,
        "call_type": &state.call_type,
        "start_time_micros": state.start_time.timestamp_micros(),
        "litellm_call_id": &state.litellm_call_id,
        "litellm_trace_id": &state.litellm_trace_id,
        "litellm_session_id": &state.litellm_session_id,
        "function_id": &state.function_id,
        "litellm_params": &state.litellm_params,
        "optional_params": &state.optional_params,
        "user": &state.user,
        "litellm_request_debug": state.litellm_request_debug,
        "stream_options": &state.stream_options,
        "custom_pricing": state.cost.custom_pricing,
        "custom_llm_provider": &state.custom_llm_provider,
        "request_logging": state.request_logging,
        "completion_start_time_micros": state.completion_start_time.map(|time| time.timestamp_micros()),
        "response_timing_metrics": &state.response_timing_metrics,
        "model_call_details": &state.model_call_details,
        "callback_dynamic_params": &state.callback_dynamic_params,
        "correlation": {
            "pre_call": &state.correlation.pre_call,
            "own": &state.correlation.own,
        },
        "context": context,
        "async_success_scheduled": state.deferred.async_success_scheduled,
    })
}

pub(super) fn context(context: &CorrelationContext) -> Value {
    serde_json::json!({
        "trace_id": &context.trace_id,
        "session_id": &context.session_id,
    })
}
