use indexmap::IndexMap;
use serde_json::Value;
use uuid::Uuid;

use betterlite_context::agentic_loop_plan::AgenticLoopPlan;

use crate::chat::{ChatMessage, MessageContent, Role};
use crate::code_execution_records::chat_dialect;
use crate::code_execution_records::code_parsing;
use crate::code_execution_records::container_extraction;
use crate::code_execution_records::responses_dialect;
use crate::code_execution_records::{CallDialect, CodeExecutionToolCall, SESSION_SCOPED_KEY};
use crate::sandbox_lifecycle::cache_operations::{
    Clock, SandboxCache, get_or_create_container, prune_expired_cache,
};
use crate::sandbox_lifecycle::lifecycle::{SandboxConfig, SandboxProvider, SandboxRegistry};
use crate::sandbox_lifecycle::resolution::{SandboxToolParams, SandboxToolRegistry};
use crate::sandbox_lifecycle::tool_execution::run_tool_call;

use super::error::PlanError;
use super::error_handling::delete_after_failure;
use super::extraction::container_identity;
use super::initialization::CodeInterpreterInterceptionLogger;
use super::plan;
use super::records::CodeInterpreterCall;
use super::request::PlanRequest;

pub async fn build_chat_plan<C, R, T, Clk>(
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
    let (tool_messages, code_interpreter_calls) = run_chat_calls(
        &provider,
        &mut logger.cache,
        &container,
        params.as_ref(),
        request,
    )
    .await?;
    let messages = chat_followup_messages(request, &tool_messages)?;
    let metadata = plan::metadata(
        sandbox_key.as_deref(),
        matches!(
            request.kwargs.get(SESSION_SCOPED_KEY),
            Some(Value::Bool(true))
        ),
        &code_interpreter_calls,
        Some("openai"),
    )?;
    let assembled = plan::assemble(request, &messages, CallDialect::ChatCompletion, metadata)?;
    Ok(assembled)
}

fn chat_followup_messages(
    request: &PlanRequest<'_>,
    tool_messages: &[ChatMessage],
) -> Result<Vec<Value>, serde_json::Error> {
    let mut messages = responses_dialect::normalize_messages(request.messages);
    messages.push(serde_json::to_value(
        chat_dialect::build_assistant_message(request.tool_calls),
    )?);
    for message in tool_messages {
        messages.push(serde_json::to_value(message)?);
    }
    Ok(messages)
}

async fn run_chat_calls<C, R, T>(
    provider: &SandboxProvider<'_, C, R, T>,
    cache: &mut SandboxCache,
    container: &Value,
    params: Option<&SandboxToolParams>,
    request: &PlanRequest<'_>,
) -> Result<(Vec<ChatMessage>, Vec<CodeInterpreterCall>), PlanError<C::Error>>
where
    C: SandboxConfig,
    R: SandboxRegistry<Error = C::Error>,
    T: SandboxToolRegistry,
{
    let container_id = container_extraction::container_id(container);
    let mut tool_messages = Vec::with_capacity(request.tool_calls.len());
    let mut code_interpreter_calls = Vec::with_capacity(request.tool_calls.len());
    for tool_call in request.tool_calls {
        let outcome =
            build_chat_tool_result(provider, params, container, tool_call, container_id.clone())
                .await;
        match outcome {
            Ok((message, call)) => {
                tool_messages.push(message);
                code_interpreter_calls.push(call);
            }
            Err(failure) => {
                let sandbox_key = container_extraction::sandbox_key(request.kwargs);
                return Err(
                    delete_after_failure(provider, cache, sandbox_key.as_deref(), failure).await,
                );
            }
        }
    }
    Ok((tool_messages, code_interpreter_calls))
}

async fn build_chat_tool_result<C, R, T>(
    provider: &SandboxProvider<'_, C, R, T>,
    params: Option<&SandboxToolParams>,
    container: &Value,
    tool_call: &CodeExecutionToolCall,
    container_id: Option<String>,
) -> Result<(ChatMessage, CodeInterpreterCall), PlanError<C::Error>>
where
    C: SandboxConfig,
    R: SandboxRegistry<Error = C::Error>,
    T: SandboxToolRegistry,
{
    let stdout = run_tool_call(provider, params, container, &tool_call.arguments).await?;
    let message = ChatMessage {
        role: Role::Tool,
        content: Some(MessageContent::Text(stdout.clone())),
        name: None,
        tool_calls: None,
        tool_call_id: Some(tool_call_identity(tool_call)),
        passthrough: IndexMap::new(),
    };
    let call = CodeInterpreterCall::completed(
        code_parsing::parse_code(&tool_call.arguments),
        container_id,
        stdout,
    );
    Ok((message, call))
}

fn tool_call_identity(tool_call: &CodeExecutionToolCall) -> String {
    tool_call
        .id
        .clone()
        .or_else(|| tool_call.call_id.clone())
        .unwrap_or_else(|| Uuid::new_v4().simple().to_string())
}

include!("chat_plan/tests/cases.rs");
