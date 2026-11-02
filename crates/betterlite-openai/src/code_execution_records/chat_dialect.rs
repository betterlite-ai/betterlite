use serde_json::Value;

use super::{CODE_EXECUTION_TOOL_NAME, CodeExecutionToolCall};
use crate::chat::response::{AssistantMessage, ChatResponse, MessageProviderFields};
use crate::chat::{FunctionCall, ToolCall};

#[must_use]
pub fn to_model_response(response: &Value) -> Option<ChatResponse> {
    if !response.is_object() {
        return None;
    }
    serde_json::from_value(response.clone()).ok()
}

#[must_use]
pub fn normalize_tool_call(tool_call: &ToolCall) -> Option<CodeExecutionToolCall> {
    if tool_call.kind != "function" || tool_call.function.name != CODE_EXECUTION_TOOL_NAME {
        return None;
    }
    Some(CodeExecutionToolCall {
        id: Some(tool_call.id.clone()),
        call_id: Some(tool_call.id.clone()),
        kind: Some("function".to_owned()),
        name: CODE_EXECUTION_TOOL_NAME.to_owned(),
        arguments: tool_call.function.arguments.clone(),
    })
}

pub fn extract_tool_calls(response: &Value) -> Vec<CodeExecutionToolCall> {
    let Some(model_response) = to_model_response(response) else {
        return Vec::new();
    };
    let Some(first_choice) = model_response.choices.first() else {
        return Vec::new();
    };
    let Some(tool_calls) = &first_choice.message.tool_calls else {
        return Vec::new();
    };
    tool_calls.iter().filter_map(normalize_tool_call).collect()
}

// LiteLLM reads only the id field here; an absent id falls back to call_id, matching how an id-less Responses-dialect CodeExecutionToolCall threaded through this chat-completion-only path would resolve
#[must_use]
pub fn build_assistant_message(tool_calls: &[CodeExecutionToolCall]) -> AssistantMessage {
    AssistantMessage {
        role: "assistant".to_owned(),
        content: None,
        tool_calls: Some(
            tool_calls
                .iter()
                .map(|tool_call| ToolCall {
                    id: tool_call
                        .id
                        .clone()
                        .or_else(|| tool_call.call_id.clone())
                        .unwrap_or_default(),
                    kind: "function".to_owned(),
                    function: FunctionCall {
                        name: CODE_EXECUTION_TOOL_NAME.to_owned(),
                        arguments: tool_call.arguments.clone(),
                    },
                })
                .collect(),
        ),
        annotations: None,
        audio: None,
        refusal: None,
        provider_specific_fields: MessageProviderFields::default(),
    }
}

include!("chat_dialect/tests/cases.rs");
