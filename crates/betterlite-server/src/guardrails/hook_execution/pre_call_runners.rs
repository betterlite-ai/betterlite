use std::sync::Arc;

use betterlite_openai::openai_base_config::Object as JsonMap;
use futures_util::future::join_all;

use super::bridge::{enrich_error, merge_metadata, record_raising_guardrail};
use super::execute::{
    CallContext, GuardrailHook, HookAnswer, HookCall, HookExecution, process_pre_call_response,
};
use super::metrics::observe;
use crate::guardrails::callback_caps::errors::{ProxyHookError, changes_request_flow};
use crate::guardrails::callback_caps::traits::{GuardrailCallback, GuardrailEvent, ProxyCallback};
use crate::guardrails::unified_guardrail::other_adapters::mcp::MCP_CALL_TYPES;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallbackRun {
    Ran,
    Skipped,
}

type ParallelRun = (JsonMap, Result<CallbackRun, ProxyHookError>);

// a scan_raw_request guardrail reads the snapshot taken before any guardrail ran
fn raw_input<'snapshot>(
    callback: &dyn ProxyCallback,
    raw_snapshot: Option<&'snapshot JsonMap>,
) -> Option<&'snapshot JsonMap> {
    raw_snapshot.filter(|_snapshot| {
        callback
            .guardrail()
            .is_some_and(GuardrailCallback::scan_raw_request)
    })
}

pub async fn process_guardrail_callback(
    execution: &HookExecution,
    callback: &dyn ProxyCallback,
    data: &mut JsonMap,
    context: CallContext<'_>,
) -> Result<CallbackRun, ProxyHookError> {
    let Some(guardrail) = callback.guardrail() else {
        return Ok(CallbackRun::Skipped);
    };
    let event = if MCP_CALL_TYPES.contains(&context.call_type) {
        GuardrailEvent::PreMcpCall
    } else {
        GuardrailEvent::PreCall
    };
    let should_run = guardrail
        .should_run_guardrail(data, event)
        .map_err(|error| ProxyHookError::Other(error.to_string()))?;
    if !should_run {
        return Ok(CallbackRun::Skipped);
    }
    let started_at = execution.clock.now();
    let mut outcome = run_pre_call(execution, callback, guardrail, data, context).await;
    match &mut outcome {
        Ok(()) => guardrail.mark_pre_call_hook_ran(data),
        Err(error) => enrich_error(execution, error, callback),
    }
    let guardrail_name = guardrail.guardrail_name().unwrap_or("unknown");
    let hook_type = GuardrailHook::PreCall.hook_type();
    observe(execution, guardrail_name, started_at, &outcome, hook_type);
    outcome.map(|()| CallbackRun::Ran)
}

// the pick of a load-balanced name runs the hook; marks, metrics and errors stay with the listed guardrail
async fn run_pre_call(
    execution: &HookExecution,
    callback: &dyn ProxyCallback,
    guardrail: &dyn GuardrailCallback,
    data: &mut JsonMap,
    context: CallContext<'_>,
) -> Result<(), ProxyHookError> {
    let call = HookCall {
        callback,
        hook: GuardrailHook::PreCall,
        context,
    };
    let answer = match guardrail.guardrail_name() {
        Some(name) if execution.balancer.should_use_load_balancing(name) => {
            execution
                .execute_with_load_balancing(name, call, data)
                .await?
        }
        Some(_) | None => execution.execute_guardrail_hook(call, data).await?,
    };
    match answer {
        HookAnswer::PreCall(response) => {
            process_pre_call_response(response, data, context.call_type)
        }
        HookAnswer::DuringCall | HookAnswer::PostCall(_) => Ok(()),
    }
}

pub async fn run_sequential(
    execution: &HookExecution,
    callback: &dyn ProxyCallback,
    data: &mut JsonMap,
    raw_snapshot: Option<&JsonMap>,
    context: CallContext<'_>,
) -> Result<(), ProxyHookError> {
    if callback
        .guardrail()
        .is_some_and(GuardrailCallback::scan_raw_request)
    {
        return run_on_raw_snapshot(execution, callback, data, raw_snapshot, context).await;
    }
    let outcome = process_guardrail_callback(execution, callback, data, context).await;
    if let Err(error) = &outcome {
        record_raising_guardrail(execution, data, callback, error);
    }
    outcome.map(|_run| ())
}

// a block-only guardrail: what it changes on its copy is dropped, only its mark reaches the live request
async fn run_on_raw_snapshot(
    execution: &HookExecution,
    callback: &dyn ProxyCallback,
    data: &mut JsonMap,
    raw_snapshot: Option<&JsonMap>,
    context: CallContext<'_>,
) -> Result<(), ProxyHookError> {
    let Some(guardrail) = callback.guardrail() else {
        return Ok(());
    };
    let mut input = raw_snapshot.unwrap_or(&*data).clone();
    let mut expected = input.clone();
    guardrail.mark_pre_call_hook_ran(&mut expected);
    let outcome = process_guardrail_callback(execution, callback, &mut input, context).await;
    let run = match outcome {
        Ok(run) => run,
        Err(error) => {
            record_raising_guardrail(execution, data, callback, &error);
            return Err(error);
        }
    };
    if run == CallbackRun::Ran {
        if input != expected {
            tracing::warn!(
                "Guardrail '{}' has scan_raw_request=True but returned a modified payload; scan_raw_request is for block-only guardrails and this mutation is being discarded. Remove scan_raw_request from this guardrail's config if it needs to mask/rewrite content.",
                guardrail
                    .guardrail_name()
                    .unwrap_or_else(|| callback.class_name())
            );
        }
        guardrail.mark_pre_call_hook_ran(data);
    }
    Ok(())
}

// a blocking error wins over a reroute, so a fast reroute never hides a slower block
pub async fn run_parallel(
    execution: &HookExecution,
    guardrails: &[Arc<dyn ProxyCallback>],
    data: &mut JsonMap,
    raw_snapshot: Option<&JsonMap>,
    context: CallContext<'_>,
) -> Result<(), ProxyHookError> {
    let shared = data.clone();
    let runs = run_together(execution, guardrails, &shared, raw_snapshot, context).await;
    let mut raised = Vec::new();
    for (callback, (input, outcome)) in guardrails.iter().zip(runs) {
        if raw_input(callback.as_ref(), raw_snapshot).is_none() {
            merge_metadata(data, &shared, &input);
        }
        let run = match outcome {
            Ok(run) => run,
            Err(error) => {
                record_raising_guardrail(execution, data, callback.as_ref(), &error);
                raised.push(error);
                continue;
            }
        };
        let scanned = callback
            .guardrail()
            .filter(|guardrail| guardrail.scan_raw_request());
        if let (CallbackRun::Ran, Some(guardrail)) = (run, scanned) {
            guardrail.mark_pre_call_hook_ran(data);
        }
    }
    if let Some(index) = raised.iter().position(|error| !changes_request_flow(error)) {
        return Err(raised.swap_remove(index));
    }
    raised.into_iter().next().map_or(Ok(()), Err)
}

// every guardrail runs to its end on its own copy, so none is left running unobserved
async fn run_together(
    execution: &HookExecution,
    guardrails: &[Arc<dyn ProxyCallback>],
    shared: &JsonMap,
    raw_snapshot: Option<&JsonMap>,
    context: CallContext<'_>,
) -> Vec<ParallelRun> {
    join_all(guardrails.iter().map(|callback| {
        let mut input = raw_input(callback.as_ref(), raw_snapshot)
            .unwrap_or(shared)
            .clone();
        async move {
            let outcome =
                process_guardrail_callback(execution, callback.as_ref(), &mut input, context).await;
            (input, outcome)
        }
    }))
    .await
}
