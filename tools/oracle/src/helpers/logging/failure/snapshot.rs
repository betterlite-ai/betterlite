use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::correlation::CorrelationContext;
use betterlite_models::decisions::Json;
use chrono::DateTime;
use serde_json::{Map, Value};

const DETAIL_KEYS: [&str; 7] = [
    "log_event_type",
    "traceback_exception",
    "original_response",
    "response_cost",
    "combined_usage_object",
    "standard_logging_object",
    "litellm_params",
];

const CLAIM_KEYS: [&str; 2] = ["has_logged_sync_failure", "has_logged_async_failure"];

pub(crate) fn details(state: &LoggingState) -> Value {
    let details = &state.model_call_details;
    let mut output = Map::new();
    for key in DETAIL_KEYS {
        let value = details.get(key).map_or(Value::Null, |found| {
            serde_json::to_value(found).unwrap_or(Value::Null)
        });
        output.insert(key.to_owned(), value);
    }
    let end_micros = details
        .get("end_time")
        .and_then(end_micros)
        .map_or(Value::Null, Value::from);
    output.insert("end_time_micros".to_owned(), end_micros);
    for claim in CLAIM_KEYS {
        output.insert(
            claim.to_owned(),
            Value::Bool(details.get(claim) == Some(&Json::Bool(true))),
        );
    }
    Value::Object(output)
}

pub(crate) fn context(context: &CorrelationContext) -> Value {
    serde_json::json!({
        "trace_id": &context.trace_id,
        "session_id": &context.session_id,
    })
}

fn end_micros(value: &Json) -> Option<i64> {
    let Json::Text(text) = value else {
        return None;
    };
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|moment| moment.timestamp_micros())
}
