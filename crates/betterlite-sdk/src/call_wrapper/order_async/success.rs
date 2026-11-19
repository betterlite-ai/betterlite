use std::sync::Arc;

use betterlite_core::rules::RuleEvaluator;

use super::AsyncRun;
use crate::call_setup::correlation::CallOutcome;
use crate::call_setup::hooks::{
    hook_call_type, run_success_hook_on_stream, success_deployment_hook,
};
use crate::call_wrapper::cache::CacheMode;
use crate::call_wrapper::error::CallError;
use crate::call_wrapper::fields::field;
use crate::call_wrapper::logging::{
    DispatchFlags, SuccessLogInput, dispatch_success_logging, success_result,
};
use crate::call_wrapper::post_call::{PostCallInput, Validation, run as post_call_processing};
use crate::call_wrapper::state::LoggingHandle;
use crate::call_wrapper::steps::{self, CallView};

pub(super) async fn stream_result<E>(
    run: &mut AsyncRun<'_, E>,
    mut outcome: CallOutcome,
    streaming_requested: bool,
) -> Result<CallOutcome, CallError>
where
    E: RuleEvaluator,
{
    super::cache::mark_stream(run);
    if !streaming_requested {
        run_success_hook_on_stream(
            &run.wrapper.setup.callbacks,
            &mut outcome,
            &run.call_kwargs,
            &run.call_type,
        )
        .await?;
    }
    let complete: Option<bool> = field(&run.request, "complete_response")?;
    if complete == Some(true) {
        return steps::assemble(run.wrapper, &run.request, run.logging.as_ref(), &outcome);
    }
    steps::update_metadata(
        run.wrapper,
        run.logging.as_ref(),
        CallView {
            request: &run.request,
            model: run.model.as_deref(),
        },
        steps::times(run.start, run.end, run.wrapper.clock.as_ref()),
        &outcome,
    );
    match &run.wrapper.cache {
        None => Ok(outcome),
        Some(cache) => Ok(cache.wrap_streaming_result(outcome, &run.call_type)),
    }
}

pub(super) fn post_call<E>(run: &AsyncRun<'_, E>, outcome: &CallOutcome) -> Result<(), CallError>
where
    E: RuleEvaluator,
{
    post_call_processing(
        &run.wrapper.rules,
        &PostCallInput {
            call_type: &run.call_type,
            model: run.model.as_deref(),
            request: &run.call_kwargs,
            outcome,
            validation: Validation {
                enable_json_schema_validation: run.wrapper.flags.enable_json_schema_validation,
                limits: run.wrapper.flags.schema_limits,
            },
        },
    )
}

pub(super) async fn finish<E>(
    run: &mut AsyncRun<'_, E>,
    outcome: CallOutcome,
) -> Result<CallOutcome, CallError>
where
    E: RuleEvaluator,
{
    let outcome = success_hook(run, outcome).await?;
    if let Some(cache) = run.wrapper.cache.as_ref().map(Arc::clone) {
        cache
            .store(&run.call_kwargs, &outcome, &run.call_type, CacheMode::Async)
            .await?;
    }
    if embedding_partial_hit(run) {
        log_success(run, &outcome);
        return Ok(combine_embedding(run, outcome));
    }
    steps::update_metadata(
        run.wrapper,
        run.logging.as_ref(),
        CallView {
            request: &run.request,
            model: run.model.as_deref(),
        },
        steps::times(run.start, run.end, run.wrapper.clock.as_ref()),
        &outcome,
    );
    log_success(run, &outcome);
    Ok(outcome)
}

// async_post_call_success_deployment_hook runs only when the call type names a CallTypes variant
async fn success_hook<E>(
    run: &AsyncRun<'_, E>,
    outcome: CallOutcome,
) -> Result<CallOutcome, CallError> {
    if hook_call_type(&run.call_type).is_none() {
        return Ok(outcome);
    }
    let CallOutcome::Response(response) = &outcome else {
        return Ok(outcome);
    };
    let hooked = success_deployment_hook(
        &run.wrapper.setup.callbacks,
        &run.call_kwargs,
        response,
        &run.call_type,
    )
    .await?;
    Ok(CallOutcome::Response(hooked))
}

// _combine_cached_embedding_response_with_api_result replaces a partial embedding hit's response with the merged one
fn embedding_partial_hit<E>(run: &AsyncRun<'_, E>) -> bool {
    matches!(run.call_type.as_str(), "embedding" | "aembedding")
        && run
            .cache
            .as_ref()
            .is_some_and(|found| found.final_embedding.is_some())
}

fn combine_embedding<E>(run: &AsyncRun<'_, E>, outcome: CallOutcome) -> CallOutcome {
    let Some(final_embedding) = run
        .cache
        .as_ref()
        .and_then(|found| found.final_embedding.as_ref())
    else {
        return outcome;
    };
    let Some(cache) = &run.wrapper.cache else {
        return outcome;
    };
    let times = steps::times(run.start, run.end, run.wrapper.clock.as_ref());
    cache.combine_cached_embedding(final_embedding, outcome, times)
}

fn log_success<E>(run: &AsyncRun<'_, E>, outcome: &CallOutcome) {
    if let Some(handle) = &run.logging {
        dispatch_success(run, handle, outcome);
    }
}

fn dispatch_success<E>(run: &AsyncRun<'_, E>, handle: &LoggingHandle, outcome: &CallOutcome) {
    let times = steps::times(run.start, run.end, run.wrapper.clock.as_ref());
    dispatch_success_logging(
        &run.wrapper.logging,
        &run.wrapper.scheduler,
        handle,
        SuccessLogInput {
            result: success_result(outcome, &run.call_type),
            times,
            flags: DispatchFlags {
                is_completion_with_fallbacks: run.is_completion_with_fallbacks,
                is_internal_call: run.internal_call,
            },
        },
    );
}
