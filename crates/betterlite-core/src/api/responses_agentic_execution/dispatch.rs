use std::sync::Arc;

use betterlite_context::agentic_callback::{
    AgenticCall, AgenticCallback, AgenticGate, AgenticLogging, AgenticLoopCall,
};
use betterlite_context::agentic_loop_plan::{AgenticLoopPlan, FollowupMessage, JsonMap};
use betterlite_models::decisions::Json;
use serde_json::value::to_raw_value;

use super::anthropic_plan::execute_anthropic_agentic_plan;
use super::error::{AgenticExecutionError, CallbackError};
use super::executor::{CallbackRun, CompletionCall, ContinuationExecutor, LoopState, PlanRun};
use super::fake_stream::maybe_wrap_in_fake_stream;
use super::finalize::finalize_refused_agentic_response;
use super::hooks::hook_message;
use super::logged_call_id;
use super::responses_plan::execute_responses_agentic_plan;
use super::values::{api_surface_name, raw_map, typed_map};
use super::{AGENTIC_LOOP_API_SURFACE_KEY, AgenticResult, CUSTOM_LLM_PROVIDER_KEY, TOOLS_KEY};
use crate::api::responses_hook_policy::agentic::get_agentic_loop_settings;
use crate::api::responses_hook_policy::error::AgenticLoopSafetyError;
use crate::api::responses_hook_policy::safety::{
    ApiSurface, can_finalize_turn, check_agentic_loop_safety,
};

enum CallbackStep {
    Done(AgenticResult),
    NoResult,
    Next,
}

// _call_agentic_completion_hooks: one dispatcher for both surfaces; only the plan path splits on surface
pub async fn call_agentic_completion_hooks<Executor>(
    call: &CompletionCall<'_>,
    callbacks: &[Arc<dyn AgenticCallback>],
    executor: &Executor,
) -> Result<Option<AgenticResult>, AgenticExecutionError>
where
    Executor: ContinuationExecutor + ?Sized,
{
    let settings = get_agentic_loop_settings(call.kwargs)?;
    let tools = parse_tools(call.optional_params)?;
    let surface_kwargs = surface_kwargs(call)?;
    let mut hook_kwargs = surface_kwargs.clone();
    hook_kwargs.insert(
        CUSTOM_LLM_PROVIDER_KEY.to_owned(),
        to_raw_value(call.custom_llm_provider)?,
    );
    let provider_kwargs = typed_map(&hook_kwargs)?;
    let logging = call.logging.map(|view| AgenticLogging {
        call_id: view.call_id.map(str::to_owned),
        agentic_loop_params: view.agentic_loop_params.cloned(),
    });
    let state = LoopState {
        settings: &settings,
        logging,
        surface_kwargs: &surface_kwargs,
        hook_kwargs: &hook_kwargs,
        provider_kwargs: &provider_kwargs,
    };
    for callback in callbacks {
        match callback_step(callback.as_ref(), call, &tools, &state, executor).await? {
            CallbackStep::Done(result) => return Ok(Some(result)),
            CallbackStep::NoResult => return Ok(None),
            CallbackStep::Next => {}
        }
    }
    let wrapped =
        maybe_wrap_in_fake_stream(call.response.to_owned(), call.logging, call.api_surface)?;
    match wrapped {
        AgenticResult::Stream(_) => Ok(Some(wrapped)),
        AgenticResult::Response(_) => Ok(None),
    }
}

async fn callback_step<Executor>(
    callback: &dyn AgenticCallback,
    call: &CompletionCall<'_>,
    tools: &[FollowupMessage],
    state: &LoopState<'_>,
    executor: &Executor,
) -> Result<CallbackStep, AgenticExecutionError>
where
    Executor: ContinuationExecutor + ?Sized,
{
    if !callback.overrides().should_run {
        return Ok(CallbackStep::Next);
    }
    let Some(gate) = gate_or_skip(callback, call, tools, state).await else {
        return Ok(CallbackStep::Next);
    };
    if !gate.run {
        return Ok(CallbackStep::Next);
    }
    let tool_calls = Json::Object(typed_map(&gate.tools)?);
    let fingerprint = match check_agentic_loop_safety(
        &tool_calls,
        &state.settings.fingerprints,
        state.settings.depth,
        state.settings.max_loops,
        call.model,
    ) {
        Ok(fingerprint) => fingerprint,
        Err(error) => return refused(call, &tool_calls, error),
    };
    let run = CallbackRun {
        call: call.clone(),
        callback,
        gate: &gate,
        tool_calls: &tool_calls,
        settings: state.settings,
        fingerprint: &fingerprint,
        hook_kwargs: state.hook_kwargs,
        provider_kwargs: state.provider_kwargs,
        logging: state.logging.as_ref(),
    };
    match plan_or_legacy(&run, executor).await {
        Ok(step) => Ok(step),
        Err(error) => {
            let call_id = logged_call_id(call.logging);
            let model = call.model;
            let message = match &error {
                CallbackError::Hook(hook) => hook_message(hook).to_owned(),
                other => other.to_string(),
            };
            tracing::error!(
                "LiteLLM.AgenticHookError: Exception in agentic completion hooks [call_id={call_id} model={model}]: {message}"
            );
            Ok(CallbackStep::Next)
        }
    }
}

async fn gate_or_skip(
    callback: &dyn AgenticCallback,
    call: &CompletionCall<'_>,
    tools: &[FollowupMessage],
    state: &LoopState<'_>,
) -> Option<AgenticGate> {
    let gate = callback
        .should_run_agentic_loop(&AgenticCall {
            response: call.response,
            model: call.model,
            messages: call.messages,
            tools: Some(tools),
            stream: call.stream,
            custom_llm_provider: call.custom_llm_provider,
            kwargs: state.surface_kwargs,
        })
        .await;
    match gate {
        Ok(ready) => Some(ready),
        Err(error) => {
            let call_id = logged_call_id(call.logging);
            let model = call.model;
            let message = hook_message(&error);
            tracing::error!(
                "LiteLLM.AgenticHookError: Exception in async_should_run_agentic_loop [call_id={call_id} model={model}]: {message}"
            );
            None
        }
    }
}

// the safety rails run outside the callback catch: responses raises, a non-streaming turn is finalized
fn refused(
    call: &CompletionCall<'_>,
    tool_calls: &Json,
    error: AgenticLoopSafetyError,
) -> Result<CallbackStep, AgenticExecutionError> {
    if !can_finalize_turn(call.stream, call.api_surface) {
        return Err(error.into());
    }
    let call_id = logged_call_id(call.logging);
    let model = call.model;
    tracing::warn!(
        "LiteLLM.AgenticLoopRefused: ending turn [call_id={call_id} model={model}]: {error}"
    );
    let finalized = finalize_refused_agentic_response(call.response, tool_calls)?;
    let wrapped =
        maybe_wrap_in_fake_stream(to_raw_value(&finalized)?, call.logging, call.api_surface)?;
    Ok(CallbackStep::Done(wrapped))
}

async fn plan_or_legacy<Executor>(
    run: &CallbackRun<'_>,
    executor: &Executor,
) -> Result<CallbackStep, CallbackError<Executor::Error>>
where
    Executor: ContinuationExecutor + ?Sized,
{
    let agentic_call = AgenticLoopCall {
        tools: &run.gate.tools,
        model: run.call.model,
        messages: run.call.messages,
        response: run.call.response,
        optional_params: run.call.optional_params,
        logging: run.logging,
        stream: run.call.stream,
        kwargs: run.hook_kwargs,
    };
    if !run.callback.overrides().build_plan {
        let response = run.callback.run_agentic_loop(&agentic_call).await?;
        let wrapped = response
            .map(|raw| maybe_wrap_in_fake_stream(raw, run.call.logging, run.call.api_surface))
            .transpose()?;
        return Ok(wrapped.map_or(CallbackStep::NoResult, CallbackStep::Done));
    }
    let plan = run.callback.build_agentic_loop_plan(&agentic_call).await?;
    plan_step(run, &plan, executor).await
}

async fn plan_step<Executor>(
    run: &CallbackRun<'_>,
    plan: &AgenticLoopPlan,
    executor: &Executor,
) -> Result<CallbackStep, CallbackError<Executor::Error>>
where
    Executor: ContinuationExecutor + ?Sized,
{
    if let Some(response) = plan.response_override.clone() {
        let wrapped = maybe_wrap_in_fake_stream(response, run.call.logging, run.call.api_surface)?;
        return Ok(CallbackStep::Done(wrapped));
    }
    if plan.terminate {
        let name = run.callback.callback_name();
        let reason = plan.stop_reason.as_deref().unwrap_or("None");
        tracing::debug!("Agentic loop terminated by callback={name} reason={reason}");
        let finalized = finalize_refused_agentic_response(run.call.response, run.tool_calls)?;
        let hooked = run
            .callback
            .post_agentic_loop_response(to_raw_value(&finalized)?, plan, run.hook_kwargs)
            .await?;
        let wrapped = maybe_wrap_in_fake_stream(hooked, run.call.logging, run.call.api_surface)?;
        return Ok(CallbackStep::Done(wrapped));
    }
    if !plan.run_agentic_loop {
        return Ok(CallbackStep::Next);
    }
    let plan_run = PlanRun { run, plan };
    let result = match run.call.api_surface {
        ApiSurface::Responses => {
            AgenticResult::Response(execute_responses_agentic_plan(&plan_run, executor).await?)
        }
        ApiSurface::AnthropicMessages => {
            let response = execute_anthropic_agentic_plan(&plan_run, executor).await?;
            maybe_wrap_in_fake_stream(response, run.call.logging, run.call.api_surface)?
        }
    };
    Ok(CallbackStep::Done(result))
}

fn surface_kwargs(call: &CompletionCall<'_>) -> Result<JsonMap, AgenticExecutionError> {
    let mut kwargs = raw_map(call.kwargs)?;
    kwargs.insert(
        AGENTIC_LOOP_API_SURFACE_KEY.to_owned(),
        to_raw_value(api_surface_name(call.api_surface))?,
    );
    Ok(kwargs)
}

// an absent or null tools key is LiteLLM's empty list, and any other shape must be the declared list
fn parse_tools(optional_params: &JsonMap) -> Result<Vec<FollowupMessage>, AgenticExecutionError> {
    let Some(raw) = optional_params.get(TOOLS_KEY) else {
        return Ok(Vec::new());
    };
    let tools = serde_json::from_str::<Option<Vec<FollowupMessage>>>(raw.get())
        .map_err(|source| AgenticExecutionError::InvalidTools { source })?;
    Ok(tools.unwrap_or_default())
}
