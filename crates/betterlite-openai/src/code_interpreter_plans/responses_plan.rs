use serde_json::{Map, Value};

use betterlite_context::agentic_loop_plan::AgenticLoopPlan;

use crate::code_execution_records::code_parsing;
use crate::code_execution_records::container_extraction;
use crate::code_execution_records::responses_dialect;
use crate::code_execution_records::{CODE_EXECUTION_TOOL_NAME, CallDialect, CodeExecutionToolCall};
use crate::sandbox_lifecycle::cache_operations::{
    Clock, SandboxCache, get_or_create_container, prune_expired_cache,
};
use crate::sandbox_lifecycle::lifecycle::{SandboxConfig, SandboxProvider, SandboxRegistry};
use crate::sandbox_lifecycle::resolution::{SandboxToolParams, SandboxToolRegistry};
use crate::sandbox_lifecycle::tool_execution::run_tool_call;

use super::error::PlanError;
use super::error_handling::delete_after_failure;
use super::extraction::{container_identity, session_scoped};
use super::initialization::CodeInterpreterInterceptionLogger;
use super::plan;
use super::records::CodeInterpreterCall;
use super::request::PlanRequest;

pub async fn build_responses_plan<C, R, T, Clk>(
    logger: &mut CodeInterpreterInterceptionLogger<C>,
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
        session_scoped(request.kwargs),
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
        input_list.push(function_call_item(tool_call));
        input_list.push(function_call_output_item(tool_call, &stdout));
        code_interpreter_calls.push(CodeInterpreterCall::completed(
            code_parsing::parse_code(&tool_call.arguments),
            container_id.clone(),
            stdout,
        ));
    }
    Ok((input_list, code_interpreter_calls))
}

fn function_call_item(tool_call: &CodeExecutionToolCall) -> Value {
    let mut item = Map::new();
    item.insert("type".to_owned(), Value::String("function_call".to_owned()));
    item.insert("call_id".to_owned(), call_id_value(tool_call));
    item.insert(
        "name".to_owned(),
        Value::String(CODE_EXECUTION_TOOL_NAME.to_owned()),
    );
    item.insert(
        "arguments".to_owned(),
        Value::String(tool_call.arguments.clone()),
    );
    Value::Object(item)
}

fn function_call_output_item(tool_call: &CodeExecutionToolCall, stdout: &str) -> Value {
    let mut item = Map::new();
    item.insert(
        "type".to_owned(),
        Value::String("function_call_output".to_owned()),
    );
    item.insert("call_id".to_owned(), call_id_value(tool_call));
    item.insert("output".to_owned(), Value::String(stdout.to_owned()));
    Value::Object(item)
}

fn call_id_value(tool_call: &CodeExecutionToolCall) -> Value {
    tool_call
        .call_id
        .as_ref()
        .map_or(Value::Null, |call_id| Value::String(call_id.clone()))
}

include!("responses_plan/tests/cases.rs");
