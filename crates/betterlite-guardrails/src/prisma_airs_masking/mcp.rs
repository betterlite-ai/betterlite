use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::value::{RawValue, to_raw_value};
use serde_with::rust::double_option;

use super::MaskingError;

#[derive(Debug)]
pub struct McpRequest {
    pub mcp_server_name: Option<String>,
    pub server_name: Option<String>,
    pub server_id: Option<String>,
    pub arguments: Option<Box<RawValue>>,
    pub mcp_arguments: Option<Box<RawValue>>,
}

type PresentOrNull = Option<Box<RawValue>>;

#[derive(Debug, Deserialize)]
struct McpRequestWire {
    mcp_server_name: Option<String>,
    server_name: Option<String>,
    server_id: Option<String>,
    #[serde(default, with = "double_option")]
    arguments: Option<PresentOrNull>,
    #[serde(default, with = "double_option")]
    mcp_arguments: Option<PresentOrNull>,
}

pub fn parse_mcp_request(bytes: &[u8]) -> Result<McpRequest, serde_json::Error> {
    let wire: McpRequestWire = serde_json::from_slice(bytes)?;
    Ok(McpRequest {
        mcp_server_name: wire.mcp_server_name,
        server_name: wire.server_name,
        server_id: wire.server_id,
        arguments: wire
            .arguments
            .map(|value| value.unwrap_or_else(|| RawValue::NULL.to_owned())),
        mcp_arguments: wire
            .mcp_arguments
            .map(|value| value.unwrap_or_else(|| RawValue::NULL.to_owned())),
    })
}

#[derive(Debug, Deserialize)]
pub struct McpServer {
    pub alias: Option<String>,
    pub server_name: Option<String>,
    pub name: Option<String>,
    pub server_id: Option<String>,
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
        request.arguments = Some(replacement.clone());
    }
    if request.mcp_arguments.is_some() {
        request.mcp_arguments = Some(replacement);
    }
    Ok(())
}

fn mcp_replacement(
    original_args: &RawValue,
    masked_text: &str,
) -> Result<Box<RawValue>, MaskingError> {
    let original = original_args.get().trim_start();
    if original.starts_with('{') || original.starts_with('[') {
        RawValue::from_string(masked_text.to_owned()).map_err(MaskingError::InvalidMcpJson)
    } else {
        to_raw_value(masked_text).map_err(MaskingError::Encoding)
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
        .filter(|name| !name.is_empty())
    {
        return name.to_owned();
    }
    if let Some(name) = request
        .server_name
        .as_deref()
        .filter(|name| !name.is_empty())
    {
        return name.to_owned();
    }
    if let Some(server) = request
        .server_id
        .as_ref()
        .and_then(|server_id| servers_by_id.get(server_id))
    {
        return [
            server.alias.as_deref(),
            server.server_name.as_deref(),
            server.name.as_deref(),
            server.server_id.as_deref(),
        ]
        .into_iter()
        .flatten()
        .find(|name| !name.is_empty())
        .unwrap_or("unknown")
        .to_owned();
    }
    names_by_tool
        .get(tool_name)
        .cloned()
        .unwrap_or_else(|| "unknown".to_owned())
}
