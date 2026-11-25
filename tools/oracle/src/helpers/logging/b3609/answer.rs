use std::sync::{Mutex, PoisonError};

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
    driven: &Driven,
) -> Value {
    let details = logging_obj
        .read(&mut |state| state.model_call_details.clone())
        .unwrap_or_default();
    let params = logging_obj
        .read(&mut |state| state.litellm_params.clone())
        .unwrap_or_default();
    let (route_calls, route_bytes) = route_summary(recorder);
    let failures = recorder
        .failures
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .len();
    let started = locked_rows(&recorder.started, |(run_id, at)| {
        json!([run_id, at.to_rfc3339()])
    });
    json!({
        "yielded": driven.yielded,
        "outcome": outcome(driven),
        "completion_start_time": completion_start(logging_obj),
        "route_calls": route_calls,
        "route_raw_bytes": route_bytes,
        "queued_jobs": driven.queued_jobs,
        "deferred_jobs": driven.deferred_jobs,
        "failure_dispatches": failures,
        "partial_usage": partial_usage(&details),
        "tinyfish_started": started,
        "tinyfish_scans": *recorder
            .scans
            .lock()
            .unwrap_or_else(PoisonError::into_inner),
        "callback_bound": callback_bound(&params),
    })
}

fn locked_rows<T, F>(list: &Mutex<Vec<T>>, row: F) -> Vec<Value>
where
    F: Fn(&T) -> Value,
{
    list.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .map(row)
        .collect()
}

fn completion_start(logging_obj: &StreamLoggingObject) -> Option<String> {
    let value = logging_obj
        .read(&mut |state| state.completion_start_time)
        .flatten()?;
    Some(value.to_rfc3339())
}

const fn outcome(driven: &Driven) -> &'static str {
    if driven.failure.is_some() {
        "failed"
    } else if driven.disconnected {
        "disconnected"
    } else {
        "completed"
    }
}

fn route_summary(recorder: &Recorder) -> (usize, Option<Vec<Vec<u8>>>) {
    let routes = recorder
        .routes
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    (routes.len(), routes.first().cloned())
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
