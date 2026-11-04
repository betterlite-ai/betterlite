mod guardrails;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::value::{RawValue, to_raw_value};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MaskingError {
    #[error("structured MCP arguments require masked JSON: {0}")]
    InvalidMcpJson(#[source] serde_json::Error),
    #[error("masked MCP JSON does not preserve the original top-level type")]
    McpTypeMismatch,
    #[error("MCP request has no rewritable arguments field")]
    MissingMcpArguments,
    #[error("failed to encode masked content: {0}")]
    Encoding(#[source] serde_json::Error),
}

#[derive(Debug, Deserialize)]
pub struct ScanResult {
    pub prompt_masked_data: Option<MaskedData>,
    pub response_masked_data: Option<MaskedData>,
}

#[derive(Debug, Deserialize)]
pub struct MaskedData {
    pub data: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RequestMessage {
    pub role: Option<String>,
    pub content: Option<MaskContent>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum MaskContent {
    Text(String),
    Parts(Vec<Box<RawValue>>),
}

#[derive(Debug, Deserialize)]
pub struct Response {
    pub choices: Option<Vec<ResponseChoice>>,
}

#[derive(Debug, Deserialize)]
pub struct ResponseChoice {
    pub message: Option<MaskedResponseMessage>,
}

#[derive(Debug, Deserialize)]
pub struct MaskedResponseMessage {
    pub content: Option<MaskContent>,
    pub tool_calls: Option<Vec<MaskedToolCall>>,
    pub function_call: Option<MaskedFunctionCall>,
}

#[derive(Debug, Deserialize)]
pub struct MaskedToolCall {
    pub function: Option<MaskedFunctionCall>,
}

#[derive(Debug, Deserialize)]
pub struct MaskedFunctionCall {
    pub arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct McpRequest {
    pub mcp_server_name: Option<String>,
    pub server_name: Option<String>,
    pub server_id: Option<String>,
    pub arguments: Option<Box<RawValue>>,
    pub mcp_arguments: Option<Box<RawValue>>,
}

#[derive(Debug, Deserialize)]
pub struct McpServer {
    pub alias: Option<String>,
    pub server_name: Option<String>,
    pub name: Option<String>,
    pub server_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TextPart {
    #[serde(rename = "type")]
    kind: Option<String>,
}

#[derive(Serialize)]
struct MaskedTextPart<'text> {
    #[serde(rename = "type")]
    kind: &'static str,
    text: &'text str,
}

#[must_use]
pub fn get_masked_text(scan_result: &ScanResult, is_response: bool) -> Option<&str> {
    let masked_data = if is_response {
        scan_result.response_masked_data.as_ref()
    } else {
        scan_result.prompt_masked_data.as_ref()
    }?;
    masked_data.data.as_deref()
}

pub fn mask_messages(
    messages: &mut [RequestMessage],
    masked_text: &str,
) -> Result<(), MaskingError> {
    let Some(message) = messages
        .iter_mut()
        .rev()
        .find(|message| message.role.as_deref() == Some("user"))
    else {
        return Ok(());
    };
    if let Some(content) = message.content.as_mut() {
        mask_content(content, masked_text)?;
    }
    Ok(())
}

pub fn mask_response(response: &mut Response, masked_text: &str) -> Result<(), MaskingError> {
    for message in response
        .choices
        .iter_mut()
        .flatten()
        .filter_map(|choice| choice.message.as_mut())
    {
        if let Some(content) = message.content.as_mut() {
            mask_content(content, masked_text)?;
        }
        for function in message
            .tool_calls
            .iter_mut()
            .flatten()
            .filter_map(|call| call.function.as_mut())
            .chain(message.function_call.iter_mut())
        {
            if function.arguments.is_some() {
                function.arguments = Some(masked_text.to_owned());
            }
        }
    }
    Ok(())
}

fn mask_content(content: &mut MaskContent, masked_text: &str) -> Result<(), MaskingError> {
    match content {
        MaskContent::Text(text) => masked_text.clone_into(text),
        MaskContent::Parts(parts) => {
            let mut masked = Vec::with_capacity(parts.len());
            for part in parts.drain(..) {
                let is_text = serde_json::from_str::<TextPart>(part.get())
                    .is_ok_and(|text_part| text_part.kind.as_deref() == Some("text"));
                if is_text {
                    masked.push(
                        to_raw_value(&MaskedTextPart {
                            kind: "text",
                            text: masked_text,
                        })
                        .map_err(MaskingError::Encoding)?,
                    );
                } else {
                    masked.push(part);
                }
            }
            *parts = masked;
        }
    }
    Ok(())
}

pub fn mask_mcp_arguments(
    request: &mut McpRequest,
    original_args: &RawValue,
    masked_text: &str,
) -> Result<(), MaskingError> {
    if request.arguments.is_none() && request.mcp_arguments.is_none() {
        return Err(MaskingError::MissingMcpArguments);
    }
    let replacement = mcp_replacement(original_args, masked_text)?;
    if request.arguments.is_some() {
        request.arguments =
            Some(RawValue::from_string(replacement.clone()).map_err(MaskingError::InvalidMcpJson)?);
    }
    if request.mcp_arguments.is_some() {
        request.mcp_arguments =
            Some(RawValue::from_string(replacement).map_err(MaskingError::InvalidMcpJson)?);
    }
    Ok(())
}

fn mcp_replacement(original_args: &RawValue, masked_text: &str) -> Result<String, MaskingError> {
    let original = original_args.get().trim_start();
    if original.starts_with('{') || original.starts_with('[') {
        let masked = serde_json::from_str::<Box<RawValue>>(masked_text)
            .map_err(MaskingError::InvalidMcpJson)?;
        let masked_kind = masked.get().trim_start();
        let valid_kind = (original.starts_with('{') && masked_kind.starts_with('{'))
            || (original.starts_with('[') && masked_kind.starts_with('['));
        if !valid_kind {
            return Err(MaskingError::McpTypeMismatch);
        }
        Ok(masked.get().to_owned())
    } else {
        serde_json::to_string(masked_text).map_err(MaskingError::Encoding)
    }
}

#[must_use]
pub fn get_mcp_server_name(
    request: &McpRequest,
    servers_by_id: &BTreeMap<String, McpServer>,
    names_by_tool: &BTreeMap<String, String>,
    tool_name: &str,
) -> String {
    if let Some(name) = request
        .mcp_server_name
        .as_deref()
        .or(request.server_name.as_deref())
        && !name.is_empty()
    {
        return name.to_owned();
    }
    if let Some(server) = request
        .server_id
        .as_ref()
        .and_then(|server_id| servers_by_id.get(server_id))
        && let Some(name) = [
            server.alias.as_deref(),
            server.server_name.as_deref(),
            server.name.as_deref(),
            server.server_id.as_deref(),
        ]
        .into_iter()
        .flatten()
        .find(|name| !name.is_empty())
    {
        return name.to_owned();
    }
    names_by_tool
        .get(tool_name)
        .cloned()
        .unwrap_or_else(|| "unknown".to_owned())
}
