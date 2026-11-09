use std::collections::BTreeMap;

use betterlite_openai::chat::ToolCall;
use betterlite_openai::chat::response::{AssistantMessage, ChatResponse};
use serde::Serialize;
use serde_json::value::RawValue;

use super::content::{McpContentBlock, SamplingRole};
use super::error::SamplingConvertError;
use super::json_object::JsonObject;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    ToolUse,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateMessageResult {
    pub role: SamplingRole,
    pub content: McpContentBlock,
    pub model: String,
    pub stop_reason: StopReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateMessageResultWithTools {
    pub role: SamplingRole,
    pub content: Vec<McpContentBlock>,
    pub model: String,
    pub stop_reason: StopReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SamplingErrorData {
    pub code: i32,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum SamplingResponse {
    Message(CreateMessageResult),
    WithTools(CreateMessageResultWithTools),
    Error(SamplingErrorData),
}

pub fn parse_tool_arguments(arguments: &str) -> Result<JsonObject, SamplingConvertError> {
    let Ok(raw) = serde_json::from_str::<Box<RawValue>>(arguments) else {
        return raw_tool_arguments(arguments);
    };
    let Ok(object) = JsonObject::try_from(raw) else {
        return Err(SamplingConvertError::ToolArgumentsNotObject);
    };
    Ok(object)
}

fn raw_tool_arguments(arguments: &str) -> Result<JsonObject, SamplingConvertError> {
    let text = serde_json::to_string(&BTreeMap::from([("raw", arguments)]))
        .map_err(|source| SamplingConvertError::ToolArgumentsEncoding { source })?;
    serde_json::from_str(&text)
        .map_err(|source| SamplingConvertError::ToolArgumentsEncoding { source })
}

pub fn convert_response(
    response: &ChatResponse,
    model_name: &str,
) -> Result<SamplingResponse, SamplingConvertError> {
    let Some(choice) = response.choices.first() else {
        return Ok(SamplingResponse::Error(SamplingErrorData {
            code: -1,
            message: format!(
                "LLM returned no choices for model '{model_name}'. This may indicate content filtering or a provider-side error."
            ),
        }));
    };
    let stop_reason = stop_reason_for(choice.finish_reason.as_deref());
    let model = if response.model.is_empty() {
        model_name
    } else {
        response.model.as_str()
    };
    let tool_calls = choice
        .message
        .tool_calls
        .as_deref()
        .filter(|calls| !calls.is_empty());
    let Some(calls) = tool_calls else {
        return Ok(SamplingResponse::Message(CreateMessageResult {
            role: SamplingRole::Assistant,
            content: McpContentBlock::Text {
                text: choice.message.content.clone().unwrap_or_default(),
            },
            model: model.to_owned(),
            stop_reason,
        }));
    };
    Ok(SamplingResponse::WithTools(tools_result(
        calls,
        &choice.message,
        model,
        stop_reason,
    )?))
}

fn tools_result(
    tool_calls: &[ToolCall],
    message: &AssistantMessage,
    model: &str,
    stop_reason: StopReason,
) -> Result<CreateMessageResultWithTools, SamplingConvertError> {
    let mut content = Vec::with_capacity(tool_calls.len().saturating_add(1));
    if let Some(text) = message.content.as_deref().filter(|text| !text.is_empty()) {
        content.push(McpContentBlock::Text {
            text: text.to_owned(),
        });
    }
    for tool_call in tool_calls {
        content.push(McpContentBlock::ToolUse {
            id: tool_call.id.clone(),
            name: tool_call.function.name.clone(),
            input: parse_tool_arguments(&tool_call.function.arguments)?,
        });
    }
    Ok(CreateMessageResultWithTools {
        role: SamplingRole::Assistant,
        content,
        model: model.to_owned(),
        stop_reason,
    })
}

fn stop_reason_for(finish_reason: Option<&str>) -> StopReason {
    match finish_reason {
        Some("tool_calls") => StopReason::ToolUse,
        Some("length") => StopReason::MaxTokens,
        _ => StopReason::EndTurn,
    }
}
