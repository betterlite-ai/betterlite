use std::sync::PoisonError;

use betterlite_core::rules::RuleEvaluator;
use betterlite_error::error::ErrorClass;
use betterlite_logging::logging_state::correlation::restore_correlation_context;
use betterlite_models::decisions::Json;
use serde_json::Number;

use super::AsyncRun;
use crate::call_setup::correlation::CallOutcome;
use crate::call_wrapper::error::CallError;
use crate::call_wrapper::retry::{
    RetryMode, context_window_model, is_router_call, num_retries, retry_strategy,
};
use crate::call_wrapper::steps;

// The except block: failure logging first, then the async retry branches, then the failure's stamps
pub(super) async fn handle<E>(
    run: &mut AsyncRun<'_, E>,
    error: CallError,
) -> Result<CallOutcome, CallError>
where
    E: RuleEvaluator,
{
    let end = run
        .deployment_end
        .unwrap_or_else(|| run.wrapper.clock.now());
    run.end = Some(end);
    if !run.internal_call {
        log_failure(run, &error).await;
    }
    let mut error = error;
    let count = num_retries(
        &mut run.request,
        run.wrapper.retry.num_retries,
        &error,
        RetryMode::Async,
    )?;
    let retried = match run.call_type.as_str() {
        "acompletion" => retry_branch(run, count, &error, true).await?,
        "aresponses" => retry_branch(run, count, &error, false).await?,
        _ => None,
    };
    if let Some(outcome) = retried {
        return Ok(outcome);
    }
    if let Some(Json::Number(number)) = run.request.get("num_retries")
        && let Some(stamped) = number.as_i64()
    {
        error.num_retries = Some(stamped);
    }
    error.timeout = run
        .request
        .get("timeout")
        .cloned()
        .filter(|value| !matches!(value, Json::Null));
    Err(error)
}

async fn log_failure<E>(run: &AsyncRun<'_, E>, error: &CallError) {
    let Some(handle) = &run.logging else {
        return;
    };
    let times = steps::times(run.start, run.end, run.wrapper.clock.as_ref());
    run.wrapper.logging.failure_sync(handle, error, times);
    run.wrapper
        .logging
        .failure_async(handle, error, times)
        .await;
}

// a truthy count retries through the target's retry entry; a failed retry is swallowed and the original error stays
async fn retry_branch<E>(
    run: &mut AsyncRun<'_, E>,
    count: Option<i64>,
    error: &CallError,
    context_fallback_allowed: bool,
) -> Result<Option<CallOutcome>, CallError>
where
    E: RuleEvaluator,
{
    if is_router_call(&run.request)? {
        return Ok(None);
    }
    let Some(count) = count.filter(|value| *value != 0) else {
        if context_fallback_allowed {
            return context_fallback(run, error).await;
        }
        return Ok(None);
    };
    run.wrapper.retry.num_retries = None;
    stamp_retry(run, count, error);
    let request = run.request.clone();
    let target = run.target;
    let logging = run.logging.clone();
    match target.retry(request, logging.as_ref()).await {
        Ok(outcome) => {
            run.chat_stream_result = outcome.is_streaming_response();
            Ok(Some(outcome))
        }
        Err(_swallowed) => Ok(None),
    }
}

fn stamp_retry<E>(run: &mut AsyncRun<'_, E>, count: i64, error: &CallError) {
    run.request
        .insert("num_retries".to_owned(), Json::Number(Number::from(count)));
    if let Some(strategy) = retry_strategy(&error.class) {
        run.request
            .insert("retry_strategy".to_owned(), Json::Text(strategy.to_owned()));
    }
}

async fn context_fallback<E>(
    run: &mut AsyncRun<'_, E>,
    error: &CallError,
) -> Result<Option<CallOutcome>, CallError>
where
    E: RuleEvaluator,
{
    if !matches!(&error.class, ErrorClass::ContextWindowExceeded(_))
        || is_router_call(&run.request)?
    {
        return Ok(None);
    }
    let Some(model) = context_window_model(&run.request, run.model.as_deref())? else {
        return Ok(None);
    };
    run.request.insert("model".to_owned(), Json::Text(model));
    let request = run.request.clone();
    let target = run.target;
    let logging = run.logging.clone();
    let outcome = target.call(request, logging.as_ref()).await?;
    run.chat_stream_result = outcome.is_streaming_response();
    Ok(Some(outcome))
}

pub(super) fn restore_correlation<E>(run: &AsyncRun<'_, E>) {
    let Some(handle) = &run.logging else {
        return;
    };
    let state = handle.state.lock().unwrap_or_else(PoisonError::into_inner);
    let mut context = handle
        .correlation
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    restore_correlation_context(&state, &mut context);
}
