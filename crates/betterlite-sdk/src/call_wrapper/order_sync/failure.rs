use betterlite_core::rules::RuleEvaluator;
use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;
use serde_json::Number;

use super::SyncRun;
use crate::call_setup::correlation::CallOutcome;
use crate::call_wrapper::error::CallError;
use crate::call_wrapper::ports::BlockingBridge;
use crate::call_wrapper::retry::{
    RetryMode, context_window_model, is_api_error, is_router_call, num_retries,
};
use crate::call_wrapper::steps;

// The except block: a completion retries or falls back, responses only retries, then failure logging and the error
pub(super) fn handle<E, B>(
    run: &mut SyncRun<'_, E, B>,
    error: CallError,
) -> Result<CallOutcome, CallError>
where
    E: RuleEvaluator,
    B: BlockingBridge,
{
    let retried = match run.call_type.as_str() {
        "completion" => retry_previous(run, &error, true)?,
        "responses" => retry_previous(run, &error, false)?,
        _ => None,
    };
    if let Some(outcome) = retried {
        return Ok(outcome);
    }
    log_failure(run, &error);
    Err(error)
}

// A context window error only falls back when the retry branch above wasn't entered
fn retry_previous<E, B>(
    run: &mut SyncRun<'_, E, B>,
    error: &CallError,
    context_fallback_allowed: bool,
) -> Result<Option<CallOutcome>, CallError>
where
    E: RuleEvaluator,
    B: BlockingBridge,
{
    let count = resolve_count(run, error)?;
    let router = is_router_call(&run.request)?;
    if let Some(count) = count.filter(|value| *value != 0) {
        if router || !is_api_error(&error.class) {
            return Ok(None);
        }
        return retry_call(run, count);
    }
    if router || !context_fallback_allowed {
        return Ok(None);
    }
    context_fallback(run, error)
}

// the request's count, then litellm.num_retries, then a retry policy; the default is cleared so a retry can't loop
fn resolve_count<E, B>(
    run: &mut SyncRun<'_, E, B>,
    error: &CallError,
) -> Result<Option<i64>, CallError>
where
    E: RuleEvaluator,
    B: BlockingBridge,
{
    let count = num_retries(
        &mut run.request,
        run.wrapper.retry.num_retries,
        error,
        RetryMode::Sync,
    )?;
    run.wrapper.retry.num_retries = None;
    Ok(count)
}

fn retry_call<E, B>(
    run: &mut SyncRun<'_, E, B>,
    count: i64,
) -> Result<Option<CallOutcome>, CallError>
where
    E: RuleEvaluator,
    B: BlockingBridge,
{
    run.request
        .insert("num_retries".to_owned(), Json::Number(Number::from(count)));
    let request = run.request.clone();
    let bridge = run.bridge;
    let target = run.target;
    let outcome = bridge.block_on(target.retry(request, run.logging.as_ref()))?;
    Ok(Some(outcome))
}

fn context_fallback<E, B>(
    run: &mut SyncRun<'_, E, B>,
    error: &CallError,
) -> Result<Option<CallOutcome>, CallError>
where
    E: RuleEvaluator,
    B: BlockingBridge,
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
    let bridge = run.bridge;
    let target = run.target;
    let outcome = bridge.block_on(target.call(request, run.logging.as_ref()))?;
    Ok(Some(outcome))
}

fn log_failure<E, B>(run: &SyncRun<'_, E, B>, error: &CallError)
where
    E: RuleEvaluator,
    B: BlockingBridge,
{
    let Some(handle) = &run.logging else {
        return;
    };
    let times = steps::times(run.start, run.end, run.wrapper.clock.as_ref());
    run.wrapper.logging.failure_sync(handle, error, times);
}
