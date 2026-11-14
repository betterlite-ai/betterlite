use serde::Serialize;
use serde_json::Value;

use betterlite_context::agentic_loop_plan::AgenticLoopPlan;

use crate::code_execution_records::code_parsing;
use crate::code_execution_records::container_extraction;
use crate::code_execution_records::responses_dialect;
use crate::code_execution_records::{
    CODE_EXECUTION_TOOL_NAME, CallDialect, CodeExecutionToolCall, SESSION_SCOPED_KEY,
};
use crate::sandbox_lifecycle::cache_operations::{
    Clock, SandboxCache, get_or_create_container, prune_expired_cache,
};
use crate::sandbox_lifecycle::lifecycle::{SandboxConfig, SandboxProvider, SandboxRegistry};
use crate::sandbox_lifecycle::resolution::{SandboxToolParams, SandboxToolRegistry};
use crate::sandbox_lifecycle::tool_execution::run_tool_call;

use super::error::PlanError;
use super::error_handling::delete_after_failure;
use super::extraction::container_identity;
use super::initialization::InterceptionLogger;
use super::plan;
use super::records::CodeInterpreterCall;
use super::request::PlanRequest;

// A Responses input item that reports the function call the sandbox ran
#[derive(Serialize)]
struct FunctionCallItem<'call> {
    #[serde(rename = "type")]
    kind: &'static str,
    call_id: Option<&'call str>,
    name: &'static str,
    arguments: &'call str,
}

// A Responses input item that carries the sandbox output back to the model
#[derive(Serialize)]
struct FunctionCallOutputItem<'call> {
    #[serde(rename = "type")]
    kind: &'static str,
    call_id: Option<&'call str>,
    output: &'call str,
}

pub async fn build_responses_plan<C, R, T, Clk>(
    logger: &mut InterceptionLogger<C>,
    registry: &R,
    tool_registry: &T,
    clock: &Clk,
    request: &PlanRequest<'_>,
) -> Result<AgenticLoopPlan, PlanError<C::Error>>
where
    C: SandboxConfig,
    R: SandboxRegistry<Error = C::Error>,
    T: SandboxToolRegistry,
    Clk: Clock,
{
    let provider = SandboxProvider {
        sandbox_config: logger.sandbox_config.as_ref(),
        registry,
        tool_registry,
        sandbox_tool_name: logger.sandbox_tool_name.as_deref(),
    };
    prune_expired_cache(&provider, &mut logger.cache, clock).await;
    let sandbox_key = container_extraction::sandbox_key(request.kwargs);
    let identity = container_identity(request.kwargs);
    let (container, params) = get_or_create_container(
        &provider,
        &mut logger.cache,
        sandbox_key.as_deref(),
        identity.as_deref(),
        clock,
    )
    .await?;
    let (input_list, code_interpreter_calls) = run_responses_calls(
        &provider,
        &mut logger.cache,
        &container,
        params.as_ref(),
        request,
    )
    .await?;
    let metadata = plan::metadata(
        sandbox_key.as_deref(),
        matches!(
            request.kwargs.get(SESSION_SCOPED_KEY),
            Some(Value::Bool(true))
        ),
        &code_interpreter_calls,
        None,
    )?;
    let assembled = plan::assemble(request, &input_list, CallDialect::Responses, metadata)?;
    Ok(assembled)
}

async fn run_responses_calls<C, R, T>(
    provider: &SandboxProvider<'_, C, R, T>,
    cache: &mut SandboxCache,
    container: &Value,
    params: Option<&SandboxToolParams>,
    request: &PlanRequest<'_>,
) -> Result<(Vec<Value>, Vec<CodeInterpreterCall>), PlanError<C::Error>>
where
    C: SandboxConfig,
    R: SandboxRegistry<Error = C::Error>,
    T: SandboxToolRegistry,
{
    let container_id = container_extraction::container_id(container);
    let mut input_list = responses_dialect::normalize_messages(request.messages);
    let mut code_interpreter_calls = Vec::with_capacity(request.tool_calls.len());
    for tool_call in request.tool_calls {
        let stdout = match run_tool_call(provider, params, container, &tool_call.arguments).await {
            Ok(stdout) => stdout,
            Err(error) => {
                let sandbox_key = container_extraction::sandbox_key(request.kwargs);
                let failure = PlanError::from(error);
                return Err(
                    delete_after_failure(provider, cache, sandbox_key.as_deref(), failure).await,
                );
            }
        };
        append_execution_items(&mut input_list, tool_call, &stdout)?;
        code_interpreter_calls.push(CodeInterpreterCall::completed(
            code_parsing::parse_code(&tool_call.arguments),
            container_id.clone(),
            stdout,
        ));
    }
    Ok((input_list, code_interpreter_calls))
}

// The model call and its sandbox output land next to each other, in the order the call ran
fn append_execution_items(
    input_list: &mut Vec<Value>,
    tool_call: &CodeExecutionToolCall,
    stdout: &str,
) -> Result<(), serde_json::Error> {
    input_list.push(serde_json::to_value(FunctionCallItem {
        kind: "function_call",
        call_id: tool_call.call_id.as_deref(),
        name: CODE_EXECUTION_TOOL_NAME,
        arguments: &tool_call.arguments,
    })?);
    input_list.push(serde_json::to_value(FunctionCallOutputItem {
        kind: "function_call_output",
        call_id: tool_call.call_id.as_deref(),
        output: stdout,
    })?);
    Ok(())
}

include!("responses_plan/tests/cases.rs");
