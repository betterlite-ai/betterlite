use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::Serialize;
use thiserror::Error;
use uuid::Uuid;

use crate::guardrail_metadata::RequestData;

pub const MCP_SESSION_ID_HEADER: &str = "mcp-session-id";
pub const DEFAULT_SERVER_NAME: &str = "litellm";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EvaluateTool {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EvaluatePayload {
    pub tool: EvaluateTool,
    #[serde(rename = "serverName")]
    pub server_name: String,
    #[serde(rename = "conversationId")]
    pub conversation_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<BTreeMap<String, GuardrailValue>>,
    #[serde(rename = "agentId", skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

#[derive(Debug, Error)]
pub enum Agent365PayloadError {
    #[error("Agent 365 evaluation data field {field} must be a string")]
    TextField { field: &'static str },
}

pub fn build_evaluate_payload(
    data: &RequestData,
    key_alias: Option<&str>,
) -> Result<EvaluatePayload, Agent365PayloadError> {
    let tool_name = text_field(data, "mcp_tool_name")?.unwrap_or_default();
    let server_name =
        text_field(data, "mcp_server_name")?.unwrap_or_else(|| DEFAULT_SERVER_NAME.to_owned());
    // LiteLLM includes the arguments only when they are a dict and omits any other shape
    let arguments = match data.get("mcp_arguments") {
        Some(GuardrailValue::Object(arguments)) => Some(arguments.clone()),
        _ => None,
    };
    let agent_id = key_alias
        .filter(|alias| !alias.is_empty())
        .map(str::to_owned);
    Ok(EvaluatePayload {
        tool: EvaluateTool { name: tool_name },
        server_name,
        conversation_id: resolve_conversation_id(data),
        arguments,
        agent_id,
    })
}

#[must_use]
pub fn resolve_conversation_id(data: &RequestData) -> String {
    tool_metadata_session(data)
        .or_else(|| header_session(data))
        .or_else(|| call_id(data))
        .unwrap_or_else(|| Uuid::new_v4().to_string())
}

fn text_field(
    data: &RequestData,
    field: &'static str,
) -> Result<Option<String>, Agent365PayloadError> {
    match data.get(field) {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(GuardrailValue::Text(text)) => Ok(Some(text.clone())),
        Some(_) => Err(Agent365PayloadError::TextField { field }),
    }
}

// The logging object's model_call_details reach the request data as mcp_tool_call_metadata, at the top level or under the request metadata.
fn tool_metadata_session(data: &RequestData) -> Option<String> {
    if let Some(session) = session_in(data.get("mcp_tool_call_metadata")) {
        return Some(session);
    }
    session_in(metadata_object(data)?.get("mcp_tool_call_metadata"))
}

fn session_in(metadata: Option<&GuardrailValue>) -> Option<String> {
    let Some(GuardrailValue::Object(fields)) = metadata else {
        return None;
    };
    match fields.get("mcp_session_id") {
        Some(GuardrailValue::Text(session)) if !session.is_empty() => Some(session.clone()),
        _ => None,
    }
}

fn header_session(data: &RequestData) -> Option<String> {
    let metadata = metadata_object(data)?;
    let Some(GuardrailValue::Object(headers)) = metadata.get("headers") else {
        return None;
    };
    let session = headers
        .iter()
        .find(|(name, _value)| name.eq_ignore_ascii_case(MCP_SESSION_ID_HEADER))
        .map(|(_name, value)| value)?;
    let GuardrailValue::Text(text) = session else {
        return None;
    };
    (!text.is_empty()).then(|| text.clone())
}

fn metadata_object(data: &RequestData) -> Option<&RequestData> {
    ["metadata", "litellm_metadata"]
        .iter()
        .find_map(|name| match data.get(*name) {
            Some(GuardrailValue::Object(metadata)) => Some(metadata),
            _ => None,
        })
}

fn call_id(data: &RequestData) -> Option<String> {
    match data.get("litellm_call_id") {
        Some(GuardrailValue::Text(call_id)) if !call_id.is_empty() => Some(call_id.clone()),
        _ => None,
    }
}

mod tests;
