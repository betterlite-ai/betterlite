use std::sync::Arc;

use betterlite_context::agentic_callback::{
    AgenticCall, AgenticCallback, AgenticGate, AgenticHookError, AgenticLoopCall,
};
use betterlite_context::agentic_loop_plan::{AgenticLoopPlan, FollowupMessage, JsonMap};
use serde_json::from_str;
use serde_json::value::to_raw_value;

use super::error::{AgenticLoopError, ChatCallError};
use super::fake_stream::FakeStreamPort;
use super::followup::{FollowupCall, execute};
use super::safety::check_loop_safety;
use super::settings::{AgenticSettings, converted_stream_requested, read_settings};
use super::{
    AGENTIC_LOOP_API_SURFACE_KEY, CHAT_COMPLETIONS_SURFACE, CUSTOM_LLM_PROVIDER_KEY,
    ChatContinuation, ChatLoopCall, ChatLoopResult, LoopPorts, TOOLS_KEY,
};

mod tests;

enum CallbackStep {
    Done(ChatLoopResult),
    NoLoop,
    Next,
}

struct CallbackRun<'run, Continuation: ?Sized, FakeStream: ?Sized> {
    call: &'run ChatLoopCall<'run>,
    gate: &'run AgenticGate,
    settings: &'run AgenticSettings,
    fingerprint: &'run str,
    hook_kwargs: &'run JsonMap,
    ports: &'run LoopPorts<'run, Continuation, FakeStream>,
}

pub async fn maybe_run_chat_agentic_loop<'call, Continuation, FakeStream>(
    call: &'call ChatLoopCall<'call>,
    callbacks: &[Arc<dyn AgenticCallback>],
    ports: &'call LoopPorts<'call, Continuation, FakeStream>,
) -> Result<Option<ChatLoopResult>, AgenticLoopError>
where
    Continuation: ChatContinuation + ?Sized,
    FakeStream: FakeStreamPort + ?Sized,
{
    let settings = read_settings(call.kwargs)?;
    let tools = parse_tools(call.optional_params)?;
    for callback in callbacks {
        match callback_step(callback.as_ref(), call, &tools, &settings, ports).await? {
            CallbackStep::Done(result) => return Ok(Some(result)),
            CallbackStep::NoLoop => return Ok(None),
            CallbackStep::Next => {}
        }
    }
    Ok(final_wrap(call, &settings, ports))
}

async fn callback_step<Continuation, FakeStream>(
    callback: &dyn AgenticCallback,
    call: &ChatLoopCall<'_>,
    tools: &[FollowupMessage],
    settings: &AgenticSettings,
    ports: &LoopPorts<'_, Continuation, FakeStream>,
) -> Result<CallbackStep, AgenticLoopError>
where
    Continuation: ChatContinuation + ?Sized,
    FakeStream: FakeStreamPort + ?Sized,
{
    if !callback.overrides().should_run {
        return Ok(CallbackStep::Next);
    }
    let hook_kwargs = loop_hook_kwargs(call.kwargs, call.custom_llm_provider)?;
    let Some(gate) = gate_or_skip(callback, call, tools, &hook_kwargs).await else {
        return Ok(CallbackStep::Next);
    };
    if !gate.run {
        return Ok(CallbackStep::Next);
    }
    let fingerprint = check_loop_safety(
        &gate.tools,
        &settings.fingerprints,
        settings.depth,
        settings.max_loops,
        call.model,
    )?;
    let run = CallbackRun {
        call,
        gate: &gate,
        settings,
        fingerprint: &fingerprint,
        hook_kwargs: &hook_kwargs,
        ports,
    };
    match plan_or_legacy(callback, &run).await {
        Ok(step) => Ok(step),
        Err(error) => {
            tracing::error!(
                "LiteLLM.AgenticHookError: Exception in chat completion agentic hooks: {}",
                error
            );
            Ok(CallbackStep::Next)
        }
    }
}

async fn gate_or_skip(
    callback: &dyn AgenticCallback,
    call: &ChatLoopCall<'_>,
    tools: &[FollowupMessage],
    hook_kwargs: &JsonMap,
) -> Option<AgenticGate> {
    match ask_gate(callback, call, tools, hook_kwargs).await {
        Ok(gate) => Some(gate),
        Err(error) => {
            tracing::error!(
                "LiteLLM.AgenticHookError: Exception in chat completion agentic gate: {}",
                error
            );
            None
        }
    }
}

async fn ask_gate(
    callback: &dyn AgenticCallback,
    call: &ChatLoopCall<'_>,
    tools: &[FollowupMessage],
    hook_kwargs: &JsonMap,
) -> Result<AgenticGate, AgenticHookError> {
    callback
        .should_run_agentic_loop(&AgenticCall {
            response: call.response,
            model: call.model,
            messages: call.messages,
            tools: Some(tools),
            stream: call.stream,
            custom_llm_provider: call.custom_llm_provider,
            kwargs: hook_kwargs,
        })
        .await
}

async fn plan_or_legacy<Continuation, FakeStream>(
    callback: &dyn AgenticCallback,
    run: &CallbackRun<'_, Continuation, FakeStream>,
) -> Result<CallbackStep, ChatCallError<Continuation::Error>>
where
    Continuation: ChatContinuation + ?Sized,
    FakeStream: FakeStreamPort + ?Sized,
{
    let loop_call = AgenticLoopCall {
        tools: &run.gate.tools,
        model: run.call.model,
        messages: run.call.messages,
        response: run.call.response,
        optional_params: run.call.optional_params,
        logging: run.call.logging.agentic(),
        stream: run.call.stream,
        kwargs: run.hook_kwargs,
    };
    if !callback.overrides().build_plan {
        let response = callback.run_agentic_loop(&loop_call).await?;
        return Ok(match response {
            Some(response) => CallbackStep::Done(ChatLoopResult::Response(response)),
            None => CallbackStep::NoLoop,
        });
    }
    let plan = callback.build_agentic_loop_plan(&loop_call).await?;
    plan_step(callback, run, &plan).await
}

async fn plan_step<Continuation, FakeStream>(
    callback: &dyn AgenticCallback,
    run: &CallbackRun<'_, Continuation, FakeStream>,
    plan: &AgenticLoopPlan,
) -> Result<CallbackStep, ChatCallError<Continuation::Error>>
where
    Continuation: ChatContinuation + ?Sized,
    FakeStream: FakeStreamPort + ?Sized,
{
    if let Some(response) = plan.response_override.clone() {
        return Ok(CallbackStep::Done(ChatLoopResult::Response(response)));
    }
    if plan.terminate {
        return Ok(CallbackStep::Done(ChatLoopResult::Response(
            run.call.response.to_owned(),
        )));
    }
    if !plan.run_agentic_loop {
        return Ok(CallbackStep::Next);
    }
    let result = execute(&FollowupCall {
        plan,
        callback,
        model: run.call.model,
        optional_params: run.call.optional_params,
        kwargs: run.call.kwargs,
        logging: run.call.logging,
        custom_llm_provider: run.call.custom_llm_provider,
        fingerprint: run.fingerprint,
        settings: run.settings,
        continuation: run.ports.continuation,
        fake_stream: run.ports.fake_stream,
    })
    .await?;
    Ok(CallbackStep::Done(result))
}

fn final_wrap<Continuation, FakeStream>(
    call: &ChatLoopCall<'_>,
    settings: &AgenticSettings,
    ports: &LoopPorts<'_, Continuation, FakeStream>,
) -> Option<ChatLoopResult>
where
    Continuation: ?Sized,
    FakeStream: FakeStreamPort + ?Sized,
{
    if !converted_stream_requested(call.kwargs) || settings.depth != 0 {
        return None;
    }
    Some(ports.fake_stream.wrap(
        ChatLoopResult::Response(call.response.to_owned()),
        call.model,
        call.custom_llm_provider,
        &call.logging,
    ))
}

// an absent or null tools key is LiteLLM's empty list, and any other shape must be the declared list
fn parse_tools(optional_params: &JsonMap) -> Result<Vec<FollowupMessage>, AgenticLoopError> {
    let Some(raw) = optional_params.get(TOOLS_KEY) else {
        return Ok(Vec::new());
    };
    let tools = from_str::<Option<Vec<FollowupMessage>>>(raw.get())
        .map_err(|source| AgenticLoopError::InvalidTools { source })?;
    Ok(tools.unwrap_or_default())
}

fn loop_hook_kwargs(
    kwargs: &JsonMap,
    custom_llm_provider: &str,
) -> Result<JsonMap, AgenticLoopError> {
    let mut hook_kwargs = kwargs.clone();
    hook_kwargs.insert(
        AGENTIC_LOOP_API_SURFACE_KEY.to_owned(),
        to_raw_value(CHAT_COMPLETIONS_SURFACE)?,
    );
    hook_kwargs.insert(
        CUSTOM_LLM_PROVIDER_KEY.to_owned(),
        to_raw_value(custom_llm_provider)?,
    );
    Ok(hook_kwargs)
}
