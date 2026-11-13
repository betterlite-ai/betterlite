use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::Serialize;

use super::RequestData;
use super::metadata;
use super::text_value;
use super::tool_calls;
use crate::generic_guardrail_contracts::{
    GuardrailInputType, GuardrailMessage, GuardrailToolParam,
};
use crate::guardrail_base::inputs::GuardrailApiInputs;

pub(super) const REQUEST_SCOPE: &str = "request";
pub(super) const RESPONSE_SCOPE: &str = "response";
pub(super) const MCP_REQUEST_SCOPE: &str = "mcp_request";
pub(super) const MCP_RESPONSE_SCOPE: &str = "mcp_response";

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub(super) enum SingulrPayload {
    Chat(ChatPayload),
    Mcp(McpPayload),
}

#[derive(Debug, Serialize)]
pub(super) struct ChatPayload {
    correlation_id: Option<String>,
    model_name: Option<String>,
    model_provider_name: Option<String>,
    guardrail_scope: &'static str,
    messages: GuardrailValue,
    images: Option<Vec<String>>,
    tools: Option<Vec<GuardrailToolParam>>,
    response: Option<AssistantMessagePayload>,
    metadata: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Serialize)]
pub(super) struct McpPayload {
    model_name: Option<String>,
    guardrail_scope: &'static str,
    tool_name: Option<String>,
    tool_arguments: GuardrailValue,
    mcp_server_name: Option<String>,
    tool_result: Option<Vec<String>>,
    metadata: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Serialize)]
pub(super) struct AssistantMessagePayload {
    role: &'static str,
    content: Option<String>,
    tool_calls: Vec<ToolCallPayload>,
}

#[derive(Debug, Serialize)]
pub(super) struct ToolCallPayload {
    id: String,
    #[serde(rename = "type")]
    call_type: String,
    function: ToolCallFunctionPayload,
}

#[derive(Debug, Serialize)]
pub(super) struct ToolCallFunctionPayload {
    name: String,
    arguments: String,
}

pub(super) fn build(
    inputs: &GuardrailApiInputs,
    request_data: &RequestData,
    input_type: GuardrailInputType,
    mcp: bool,
) -> Option<SingulrPayload> {
    match input_type {
        GuardrailInputType::Request if mcp => Some(SingulrPayload::Mcp(mcp_request(request_data))),
        GuardrailInputType::Request => chat_request(inputs, request_data),
        GuardrailInputType::Response if mcp => {
            let texts = inputs.texts.as_deref().unwrap_or_default();
            if texts.is_empty() {
                return None;
            }
            Some(SingulrPayload::Mcp(McpPayload {
                model_name: request_data
                    .get("model")
                    .and_then(text_value)
                    .map(str::to_owned),
                guardrail_scope: MCP_RESPONSE_SCOPE,
                tool_name: None,
                tool_arguments: GuardrailValue::Null,
                mcp_server_name: None,
                tool_result: Some(texts.to_vec()),
                metadata: metadata::build(request_data),
            }))
        }
        GuardrailInputType::Response => {
            Some(SingulrPayload::Chat(chat_response(inputs, request_data)))
        }
    }
}

fn chat_request(inputs: &GuardrailApiInputs, request_data: &RequestData) -> Option<SingulrPayload> {
    let structured = inputs.structured_messages.as_deref().unwrap_or_default();
    let texts = inputs.texts.as_deref().unwrap_or_default();
    let messages: Vec<GuardrailValue> = if structured.is_empty() {
        texts.iter().map(|text| user_message(text)).collect()
    } else {
        structured.iter().map(message_value).collect()
    };
    let images = inputs.images.as_ref().filter(|images| !images.is_empty());
    let tools = inputs.tools.as_ref().filter(|tools| !tools.is_empty());
    if messages.is_empty() && images.is_none() && tools.is_none() {
        return None;
    }
    Some(SingulrPayload::Chat(ChatPayload {
        correlation_id: request_data
            .get("litellm_call_id")
            .and_then(text_value)
            .map(str::to_owned),
        model_name: inputs.model.clone(),
        model_provider_name: None,
        guardrail_scope: REQUEST_SCOPE,
        messages: GuardrailValue::Array(messages),
        images: inputs.images.clone(),
        tools: inputs.tools.clone(),
        response: None,
        metadata: metadata::build(request_data),
    }))
}

fn chat_response(inputs: &GuardrailApiInputs, request_data: &RequestData) -> ChatPayload {
    let texts = inputs.texts.as_deref().unwrap_or_default();
    let content = (!texts.is_empty()).then(|| texts.join("\n"));
    let tool_calls = inputs
        .tool_calls
        .as_deref()
        .unwrap_or_default()
        .iter()
        .filter_map(tool_calls::convert)
        .collect();
    ChatPayload {
        correlation_id: request_data
            .get("litellm_call_id")
            .and_then(text_value)
            .map(str::to_owned),
        model_name: request_data
            .get("model")
            .and_then(text_value)
            .map(str::to_owned),
        model_provider_name: None,
        guardrail_scope: RESPONSE_SCOPE,
        messages: request_data
            .get("messages")
            .cloned()
            .unwrap_or(GuardrailValue::Null),
        images: inputs.images.clone(),
        tools: None,
        response: Some(AssistantMessagePayload {
            role: "assistant",
            content,
            tool_calls,
        }),
        metadata: metadata::build(request_data),
    }
}

fn mcp_request(request_data: &RequestData) -> McpPayload {
    let tool_arguments = match request_data.get("mcp_arguments") {
        Some(GuardrailValue::Null) | None => request_data
            .get("arguments")
            .cloned()
            .unwrap_or(GuardrailValue::Null),
        Some(arguments) => arguments.clone(),
    };
    McpPayload {
        model_name: None,
        guardrail_scope: MCP_REQUEST_SCOPE,
        tool_name: request_data
            .get("mcp_tool_name")
            .and_then(text_value)
            .or_else(|| request_data.get("name").and_then(text_value))
            .map(str::to_owned),
        tool_arguments,
        mcp_server_name: request_data
            .get("mcp_server_name")
            .and_then(text_value)
            .map(str::to_owned),
        tool_result: None,
        metadata: metadata::build(request_data),
    }
}

fn message_value(message: &GuardrailMessage) -> GuardrailValue {
    let mut fields = BTreeMap::new();
    fields.insert(
        "role".to_owned(),
        GuardrailValue::Text(message.role.clone()),
    );
    for (key, value) in &message.extra {
        fields.insert(key.clone(), value.clone());
    }
    GuardrailValue::Object(fields)
}

fn user_message(text: &str) -> GuardrailValue {
    let mut fields = BTreeMap::new();
    fields.insert("role".to_owned(), GuardrailValue::Text("user".to_owned()));
    fields.insert("content".to_owned(), GuardrailValue::Text(text.to_owned()));
    GuardrailValue::Object(fields)
}
