use super::super::metadata::wire::{MetadataError, MetadataMap, payload::Payload};
use super::super::utils::coercion::OtelModelValue;
use super::{RequestContext, RequestIdentity, error::SpanError, helpers};
use crate::otel::vocabulary::constants::{GenAIOperation, MCPMethod};
use crate::otel::vocabulary::resolvers::resolve_operation;
use rust_decimal::Decimal;

#[derive(Clone, Debug)]
pub struct MCPToolCallSpanData {
    pub operation: GenAIOperation,
    pub method: MCPMethod,
    pub tool_name: String,
    pub server_name: Option<String>,
    pub server_address: Option<String>,
    pub server_port: Option<u16>,
    pub session_id: Option<String>,
    pub arguments_json: Option<String>,
    pub result_json: Option<String>,
    pub error: Option<SpanError>,
    pub response_cost: Option<Decimal>,
    pub identity: RequestIdentity,
}

impl MCPToolCallSpanData {
    pub fn from_standard_logging_payload(
        payload: &Payload,
        capture_content: bool,
    ) -> Result<Self, MetadataError> {
        let metadata = tool_call_metadata(payload);
        let resource = metadata
            .and_then(|fields| fields.get("mcp_server_resource"))
            .and_then(OtelModelValue::as_str);
        let (server_address, server_port) = upstream_address_port(resource);
        Ok(Self {
            operation: resolve_operation(call_type(payload).unwrap_or_default()),
            method: MCPMethod::ToolsCall,
            tool_name: metadata_text(metadata, "name").unwrap_or_default(),
            server_name: metadata_text(metadata, "mcp_server_name"),
            server_address,
            server_port,
            session_id: metadata_text(metadata, "mcp_session_id"),
            arguments_json: captured_json(metadata, "arguments", capture_content),
            result_json: captured_json(metadata, "result", capture_content),
            error: SpanError::from_payload(&payload.extra),
            response_cost: payload.response_cost,
            identity: RequestContext::from_standard_logging_payload(payload)?.identity,
        })
    }
}

#[derive(Clone, Debug)]
pub struct MCPListToolsSpanData {
    pub method: MCPMethod,
    pub session_id: Option<String>,
    pub error: Option<SpanError>,
    pub identity: RequestIdentity,
}

impl MCPListToolsSpanData {
    pub fn from_standard_logging_payload(payload: &Payload) -> Result<Self, MetadataError> {
        Ok(Self {
            method: MCPMethod::ToolsList,
            session_id: None,
            error: SpanError::from_payload(&payload.extra),
            identity: RequestContext::from_standard_logging_payload(payload)?.identity,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PayloadKind {
    McpToolCall,
    McpListTools,
    Inference,
}

impl PayloadKind {
    #[must_use]
    pub fn from_payload(payload: &Payload) -> Self {
        if tool_call_metadata(payload).is_some() || call_type(payload) == Some("call_mcp_tool") {
            return Self::McpToolCall;
        }
        if call_type(payload) == Some("list_mcp_tools") {
            return Self::McpListTools;
        }
        Self::Inference
    }
}

// the scheme default keeps a consumer that keys a dependency off the address from rendering port 0
fn upstream_address_port(resource: Option<&str>) -> (Option<String>, Option<u16>) {
    let Some(text) = resource.filter(|text| !text.is_empty()) else {
        return (None, None);
    };
    let Some((scheme, remainder)) = text.split_once("://") else {
        return (None, None);
    };
    let authority = remainder.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.contains('[') != authority.contains(']') {
        return (None, None);
    }
    let Some((host, port_text)) = host_and_port(authority) else {
        return (None, None);
    };
    if host.is_empty() {
        return (None, None);
    }
    let port = match port_text.filter(|digits| !digits.is_empty()) {
        None => default_port(scheme),
        Some(digits) if !digits.bytes().all(|byte| byte.is_ascii_digit()) => return (None, None),
        Some(digits) => match digits.parse::<u16>() {
            // LiteLLM writes `port or default_port`, so an explicit 0 also takes the default
            Ok(0) => default_port(scheme),
            Ok(port) => Some(port),
            Err(_) => return (None, None),
        },
    };
    (Some(host.to_lowercase()), port)
}

fn default_port(scheme: &str) -> Option<u16> {
    match scheme.to_lowercase().as_str() {
        "https" => Some(443),
        "http" => Some(80),
        _ => None,
    }
}

fn host_and_port(authority: &str) -> Option<(&str, Option<&str>)> {
    let host_info = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    if let Some(bracketed) = host_info.strip_prefix('[') {
        let (host, suffix) = bracketed.split_once(']')?;
        return Some((host, suffix.strip_prefix(':')));
    }
    let host_port = host_info
        .split_once(':')
        .map_or((host_info, None), |(host, port)| (host, Some(port)));
    Some(host_port)
}

fn tool_call_metadata(payload: &Payload) -> Option<&MetadataMap> {
    payload
        .metadata
        .extra
        .get("mcp_tool_call_metadata")
        .and_then(OtelModelValue::as_object)
        // an empty metadata object is not a tool call, matching LiteLLM's truthiness test
        .filter(|fields| !fields.is_empty())
}

fn metadata_text(metadata: Option<&MetadataMap>, key: &str) -> Option<String> {
    metadata
        .and_then(|fields| fields.get(key))
        .and_then(OtelModelValue::as_str)
        .map(str::to_owned)
}

fn captured_json(
    metadata: Option<&MetadataMap>,
    key: &str,
    capture_content: bool,
) -> Option<String> {
    if !capture_content {
        return None;
    }
    metadata
        .and_then(|fields| fields.get(key))
        .filter(|value| !matches!(value, OtelModelValue::Null(())))
        .and_then(helpers::json_or_none)
}

fn call_type(payload: &Payload) -> Option<&str> {
    payload
        .extra
        .get("call_type")
        .and_then(OtelModelValue::as_str)
}

#[path = "mcp_call/tests.rs"]
mod tests;
