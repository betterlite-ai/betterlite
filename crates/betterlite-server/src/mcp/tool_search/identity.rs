use std::collections::BTreeMap;

use crate::mcp::catalog_guard::types::MCPTool;
use betterlite_db::mcp_stored_contracts::JsonValue;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use thiserror::Error;

use super::MCPToolSearchHit;

const MCP_PROXY_IDENTITY_META_KEY: &str = "litellm.ai/proxy_tool_identity";

#[derive(Debug, Error, Eq, PartialEq)]
pub enum MCPProxyToolIdentityError {
    #[error("MCP proxy tool identity is missing")]
    Missing,
    #[error("MCP proxy tool identity is invalid")]
    Invalid,
    #[error("MCP proxy tool identity is invalid: {field}")]
    InvalidField { field: &'static str },
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MCPProxySearchResult {
    pub tool_id: String,
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MCPProxySchemaResult {
    #[serde(rename = "tool_id")]
    pub tool_id: String,
    pub name: String,
    pub description: String,
    pub input_schema: BTreeMap<String, JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<BTreeMap<String, JsonValue>>,
}

#[must_use]
pub fn with_mcp_proxy_identity(tool: &MCPTool, server_id: &str) -> MCPTool {
    let mut identified = tool.clone();
    let mut metadata = identified.meta.take().unwrap_or_default();
    metadata.insert(
        MCP_PROXY_IDENTITY_META_KEY.to_owned(),
        JsonValue::Object(BTreeMap::from([
            (
                "server_id".to_owned(),
                JsonValue::String(server_id.to_owned()),
            ),
            ("tool_name".to_owned(), JsonValue::String(tool.name.clone())),
        ])),
    );
    identified.meta = Some(metadata);
    identified
}

pub fn mcp_proxy_tool_id(tool: &MCPTool) -> Result<String, MCPProxyToolIdentityError> {
    let identity = tool
        .meta
        .as_ref()
        .and_then(|metadata| metadata.get(MCP_PROXY_IDENTITY_META_KEY))
        .ok_or(MCPProxyToolIdentityError::Missing)?;
    let JsonValue::Object(fields) = identity else {
        return Err(MCPProxyToolIdentityError::Invalid);
    };
    let server_id = fields
        .get("server_id")
        .and_then(as_json_string)
        .ok_or(MCPProxyToolIdentityError::InvalidField { field: "server_id" })?;
    let tool_name = fields
        .get("tool_name")
        .and_then(as_json_string)
        .ok_or(MCPProxyToolIdentityError::InvalidField { field: "tool_name" })?;
    let mut digest = Sha256::new();
    digest.update(server_id.as_bytes());
    digest.update([0]);
    digest.update(tool_name.as_bytes());
    let encoded = hex::encode(digest.finalize());
    Ok(encoded.chars().take(32).collect())
}

fn as_json_string(value: &JsonValue) -> Option<&str> {
    match value {
        JsonValue::String(text) => Some(text),
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::Array(_)
        | JsonValue::Object(_) => None,
    }
}

pub fn proxy_search_result(
    hit: &MCPToolSearchHit,
) -> Result<MCPProxySearchResult, MCPProxyToolIdentityError> {
    let fields = proxy_tool_fields(&hit.tool)?;
    Ok(MCPProxySearchResult {
        tool_id: fields.tool_id,
        name: fields.name,
        description: fields.description,
        score: hit.score,
    })
}

pub fn proxy_schema_result(
    tool: &MCPTool,
) -> Result<MCPProxySchemaResult, MCPProxyToolIdentityError> {
    let fields = proxy_tool_fields(tool)?;
    Ok(MCPProxySchemaResult {
        tool_id: fields.tool_id,
        name: fields.name,
        description: fields.description,
        input_schema: tool.input_schema.clone(),
        output_schema: tool.output_schema.clone(),
    })
}

struct ProxyToolFields {
    tool_id: String,
    name: String,
    description: String,
}

fn proxy_tool_fields(tool: &MCPTool) -> Result<ProxyToolFields, MCPProxyToolIdentityError> {
    Ok(ProxyToolFields {
        tool_id: mcp_proxy_tool_id(tool)?,
        name: tool.name.clone(),
        description: tool.description.clone().unwrap_or_default(),
    })
}
