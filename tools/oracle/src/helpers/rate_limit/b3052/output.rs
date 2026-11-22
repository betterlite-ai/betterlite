use std::collections::BTreeMap;
use std::sync::PoisonError;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_rate_limit::dynamic_admission::admission::CheckRateLimitError;
use betterlite_rate_limit::request_stash::RequestStashSlot;
use serde_json::{Value, json};

use super::{RecordedCall, StubLimiter};

// the recorded calls in the order the handler made them, split by kind so the cases can compare each list
fn calls_json(calls: &[RecordedCall]) -> Result<(Vec<Value>, Vec<Value>), serde_json::Error> {
    let mut atomic = Vec::new();
    let mut tracking = Vec::new();
    for recorded in calls {
        match recorded {
            RecordedCall::Atomic(atomic_call) => atomic.push(json!({
                "descriptors": serde_json::to_value(&atomic_call.descriptors)?,
                "increments": atomic_call
                    .increments
                    .iter()
                    .map(|amount| json!({"requests": amount.requests, "tokens": amount.tokens}))
                    .collect::<Vec<Value>>(),
            })),
            RecordedCall::Tracking(tracking_call) => tracking.push(json!({
                "descriptors": serde_json::to_value(&tracking_call.descriptors)?,
                "read_only": tracking_call.read_only,
            })),
        }
    }
    Ok((atomic, tracking))
}

fn stash_json(stash: &RequestStashSlot) -> Value {
    let Some(stored) = stash.get() else {
        return Value::Null;
    };
    let response = {
        let state = stored.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.rate_limit_response.clone()
    };
    response.map_or(Value::Null, |stored_response| {
        json!({
            "overall_code": stored_response.overall_code,
            "statuses": stored_response.statuses,
        })
    })
}

fn headers_json(failure: &CheckRateLimitError) -> BTreeMap<String, String> {
    match failure {
        CheckRateLimitError::Rejection(error) => error
            .headers
            .iter()
            .flat_map(|map| map.iter())
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|text| (name.as_str().to_owned(), text.to_owned()))
            })
            .collect(),
        CheckRateLimitError::Denial(_) | CheckRateLimitError::Limiter(_) => BTreeMap::new(),
    }
}

fn unexpected(message: &str) -> HelperResult {
    Err(Box::new(HelperError {
        class: "UnexpectedFailure".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    }))
}

pub(super) fn answer(
    outcome: &Result<(), CheckRateLimitError>,
    limiter: &StubLimiter,
    stash: &RequestStashSlot,
) -> HelperResult {
    let (atomic_calls, tracking_calls) = {
        let calls = limiter.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls_json(&calls)?
    };
    let error = match outcome {
        Ok(()) => Value::Null,
        Err(failure @ CheckRateLimitError::Rejection(rejection)) => json!({
            "status": 429_u16,
            "detail": rejection.detail.clone(),
            "headers": headers_json(failure),
        }),
        Err(other @ (CheckRateLimitError::Denial(_) | CheckRateLimitError::Limiter(_))) => {
            return unexpected(&other.to_string());
        }
    };
    Ok(json!({
        "atomic_calls": atomic_calls,
        "tracking_calls": tracking_calls,
        "stash": stash_json(stash),
        "error": error,
    }))
}
