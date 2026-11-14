use serde_json::{Map, Value};

use crate::code_execution_records::{INTERCEPTION_ACTIVE_KEY, chat_dialect, responses_dialect};

use super::initialization::CodeInterpreterInterceptionLogger;
use super::records::CodeExecutionToolCalls;

pub const CHAT_COMPLETION_AGENTIC_SURFACE: &str = "chat_completions";

// The surface marker LiteLLM branches on; anything but the chat value takes the Responses path
#[must_use]
pub fn is_chat_completion_surface(kwargs: &Map<String, Value>) -> bool {
    kwargs
        .get("_agentic_loop_api_surface")
        .and_then(Value::as_str)
        == Some(CHAT_COMPLETION_AGENTIC_SURFACE)
}

// The arguments LiteLLM's async_should_run_agentic_loop reads; the model, messages, tools and stream it ignores
pub struct GateRequest<'a> {
    pub response: &'a Value,
    pub custom_llm_provider: &'a str,
    pub kwargs: &'a Map<String, Value>,
}

#[must_use]
pub fn should_run_agentic_loop<C>(
    logger: &CodeInterpreterInterceptionLogger<C>,
    request: &GateRequest<'_>,
) -> Option<CodeExecutionToolCalls> {
    if !logger.enabled {
        return None;
    }
    if !matches!(
        request.kwargs.get(INTERCEPTION_ACTIVE_KEY),
        Some(Value::Bool(true))
    ) {
        return None;
    }
    if let Some(providers) = &logger.enabled_providers
        && !providers
            .iter()
            .any(|provider| provider.as_str() == request.custom_llm_provider)
    {
        return None;
    }
    let tool_calls = if is_chat_completion_surface(request.kwargs) {
        chat_dialect::extract_tool_calls(request.response)
    } else {
        responses_dialect::extract_tool_calls(request.response)
    };
    if tool_calls.is_empty() {
        return None;
    }
    Some(CodeExecutionToolCalls { tool_calls })
}

include!("gate/tests/cases.rs");
