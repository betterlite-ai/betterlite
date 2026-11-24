use std::sync::PoisonError;

use betterlite_logging::passthrough_stream_lifecycle::failure::StreamFailure;
use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::{Value, json};

use super::fakes::Recorder;

pub(super) struct Driven {
    pub(super) yielded: Vec<Vec<u8>>,
    pub(super) failure: Option<StreamFailure>,
    pub(super) disconnected: bool,
    pub(super) queued_jobs: usize,
    pub(super) deferred_jobs: usize,
}

pub(super) fn answer(
    logging_obj: &StreamLoggingObject,
    recorder: &Recorder,
    driven: Driven,
) -> Value {
    let details = logging_obj
        .read(&mut |state| state.model_call_details.clone())
        .unwrap_or_default();
    let params = logging_obj
        .read(&mut |state| state.litellm_params.clone())
        .unwrap_or_default();
    let routes = recorder
        .routes
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let route_calls = routes.len();
    let route_bytes = routes.first().cloned();
    drop(routes);
    let completion = logging_obj
        .read(&mut |state| state.completion_start_time.map(|value| value.to_rfc3339()))
        .flatten();
    let outcome = if driven.failure.is_some() {
        "failed"
    } else if driven.disconnected {
        "disconnected"
    } else {
        "completed"
    };
    json!({
        "yielded": driven.yielded,
        "outcome": outcome,
        "error": driven
            .failure
            .as_ref()
            .map(|failure| format!("{}: {}", failure.class, failure.message)),
        "completion_start_time": completion,
        "route_calls": route_calls,
        "route_raw_bytes": route_bytes,
        "queued_jobs": driven.queued_jobs,
        "deferred_jobs": driven.deferred_jobs,
        "failures": recorder
            .failures
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|failure| json!({
                "class": failure.class,
                "message": failure.message,
                "traceback_tail": failure.traceback.lines().last().unwrap_or_default(),
            }))
            .collect::<Vec<_>>(),
        "partial_usage": partial_usage(&details),
        "tinyfish_started": recorder
            .started
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|(run_id, at)| json!([run_id, at.to_rfc3339()]))
            .collect::<Vec<_>>(),
        "tinyfish_scans": *recorder
            .scans
            .lock()
            .unwrap_or_else(PoisonError::into_inner),
        "callback_bound": callback_bound(&params),
    })
}

fn partial_usage(details: &IndexMap<String, Json>) -> Option<Value> {
    let Json::Object(usage) = details.get("combined_usage_object")? else {
        return None;
    };
    let Json::Number(cost) = details.get("response_cost")? else {
        return None;
    };
    let prompt = tokens(usage, "prompt_tokens")?;
    let completion = tokens(usage, "completion_tokens")?;
    let total = tokens(usage, "total_tokens")?;
    Some(json!({
        "prompt_tokens": prompt,
        "completion_tokens": completion,
        "total_tokens": total,
        "response_cost": cost.to_string(),
    }))
}

fn tokens(usage: &IndexMap<String, Json>, key: &str) -> Option<i64> {
    let Some(Json::Number(number)) = usage.get(key) else {
        return None;
    };
    number.as_i64()
}

fn callback_bound(params: &IndexMap<String, Json>) -> Option<bool> {
    let Json::Object(metadata) = params.get("metadata")? else {
        return None;
    };
    let Json::Object(reservation) = metadata.get("user_api_key_budget_reservation")? else {
        return None;
    };
    let Json::Bool(flag) = reservation.get("callback_bound")? else {
        return None;
    };
    Some(*flag)
}
