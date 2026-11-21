use betterlite_core::routes::CallType;
use betterlite_openai::openai_base_config::Object as JsonMap;
use futures_util::StreamExt as _;
use futures_util::stream::FuturesUnordered;

use super::bridge::merge_metadata;
use super::execute::{CallContext, GuardrailHook, HookCall, HookExecution, HookPath, hook_path};
use super::metrics::run_guardrail_with_metrics;
use crate::guardrails::callback_caps::caps::CallbackCapabilities;
use crate::guardrails::callback_caps::errors::ProxyHookError;
use crate::guardrails::callback_caps::traits::{GuardrailEvent, ProxyCallback};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DuringCallTask {
    Moderation,
    Guardrail,
}

// which hook a callback runs during the call, if any
fn during_call_task(
    callback: &dyn ProxyCallback,
    data: &JsonMap,
    context: CallContext<'_>,
) -> Result<Option<DuringCallTask>, ProxyHookError> {
    let Some(guardrail) = callback.guardrail() else {
        let moderates =
            callback.overrides().request.moderation && context.user_api_key_dict.is_some();
        return Ok(moderates.then_some(DuringCallTask::Moderation));
    };
    // LiteLLM's v1 guardrails: without an event_hook the moderation_check decides and the gate is not asked
    if !guardrail.event_hook_is_set()
        && let Some(check) = guardrail.moderation_check()
    {
        return Ok((check != "pre_call").then_some(DuringCallTask::Guardrail));
    }
    let event = if context.call_type == CallType::CallMcpTool {
        GuardrailEvent::DuringMcpCall
    } else {
        GuardrailEvent::DuringCall
    };
    let runs = guardrail
        .should_run_guardrail(data, event)
        .map_err(|error| ProxyHookError::Other(error.to_string()))?;
    Ok(runs.then_some(DuringCallTask::Guardrail))
}

// every selected hook starts at once; the first error to come back is raised and the rest are dropped
pub async fn during_call_hook(
    execution: &HookExecution,
    capabilities: &CallbackCapabilities,
    data: &mut JsonMap,
    context: CallContext<'_>,
) -> Result<(), ProxyHookError> {
    let flags = capabilities.guardrails;
    if !flags.has_guardrail && !flags.has_moderation_override {
        return Ok(());
    }
    let mut selected = Vec::new();
    for callback in &capabilities.resolved_callbacks {
        if let Some(task) = during_call_task(callback.as_ref(), data, context)? {
            selected.push((callback, task));
        }
    }
    let shared = data.clone();
    let mut pending: FuturesUnordered<_> = selected
        .into_iter()
        .map(|(callback, task)| {
            let mut input = shared.clone();
            async move {
                let outcome =
                    run_during_call(execution, callback.as_ref(), task, &mut input, context).await;
                (input, outcome)
            }
        })
        .collect();
    while let Some((input, outcome)) = pending.next().await {
        merge_metadata(data, &shared, &input);
        outcome?;
    }
    Ok(())
}

// the unified moderation hook needs the caller's key; without it the class's own hook runs
async fn run_during_call(
    execution: &HookExecution,
    callback: &dyn ProxyCallback,
    task: DuringCallTask,
    data: &mut JsonMap,
    context: CallContext<'_>,
) -> Result<(), ProxyHookError> {
    if task == DuringCallTask::Moderation {
        return callback
            .moderation(data, context.user_api_key_dict, context.call_type)
            .await;
    }
    let hook = GuardrailHook::DuringCall;
    let path = if context.user_api_key_dict.is_some() {
        hook_path(callback, &hook)
    } else {
        HookPath::Own
    };
    let call = HookCall {
        callback,
        hook,
        context,
    };
    run_guardrail_with_metrics(execution, call, path, data)
        .await
        .map(|_answer| ())
}
