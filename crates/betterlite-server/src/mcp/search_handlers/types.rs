use std::collections::BTreeMap;

use betterlite_auth::user_roles::UserRole;
use betterlite_core::skills::search::SkillSearchResult;
use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_mcp::request_headers::raw_headers::RawHeaders;
use secrecy::SecretString;

use crate::agent_endpoints::search::AgentSearchResult;
use crate::mcp::catalog_guard::types::MCPTool;
use crate::mcp::hook_payloads::GuardrailContext;
use crate::mcp::tool_search::identity::MCPProxyToolIdentityError;

use super::ports::{McpOperations, SearchPorts, SearchSettings};

// LiteLLM's proxy_tool_identity meta key, read back out of a listed tool
const PROXY_IDENTITY_META_KEY: &str = "litellm.ai/proxy_tool_identity";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CallerScope {
    pub user_id: Option<String>,
    pub team_id: Option<String>,
    pub end_user_id: Option<String>,
    pub api_key_hash: Option<String>,
    pub role: Option<UserRole>,
}

#[derive(Clone, Debug, Default)]
pub struct McpRequestScope {
    pub client_ip: Option<String>,
    pub mcp_servers: Option<Vec<String>>,
    pub mcp_auth_header: Option<SecretString>,
    pub mcp_server_auth_headers: BTreeMap<String, BTreeMap<String, SecretString>>,
    pub oauth2_headers: BTreeMap<String, SecretString>,
    pub raw_headers: RawHeaders,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessibleAgent {
    pub agent_id: String,
    pub agent_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessibleSkill {
    pub skill_id: String,
    pub display_title: Option<String>,
    pub description: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchOutcome<T> {
    Hits(Vec<T>),
    NotConfigured { reason: String },
    EmbeddingFailed { reason: String },
}

pub type AgentSearchOutcome = SearchOutcome<AgentSearchResult>;
pub type SkillSearchOutcome = SearchOutcome<SkillSearchResult>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProxyToolIdentity {
    pub server_id: String,
    pub tool_name: String,
}

#[derive(Clone, Copy, Debug)]
pub struct AgentSearchRequest<'request> {
    pub query: &'request str,
    pub top_k: i64,
}

#[derive(Clone, Copy, Debug)]
pub struct SkillSearchRequest<'request> {
    pub query: &'request str,
    pub top_k: i64,
}

#[derive(Clone, Copy, Debug)]
pub struct McpToolSearchRequest<'request> {
    pub query: &'request str,
    pub top_k: i64,
}

#[derive(Clone, Copy, Debug)]
pub struct ProxyToolRequest<'request> {
    pub name: &'request str,
    pub arguments: &'request BTreeMap<String, JsonValue>,
}

#[derive(Clone, Copy, Debug)]
pub struct ToolCallRequest<'call> {
    pub tool_name: &'call str,
    pub arguments: &'call BTreeMap<String, JsonValue>,
    pub requested_server_id: Option<&'call str>,
    pub guardrail_context: Option<&'call GuardrailContext>,
}

pub struct HandlerContext<'handler> {
    pub caller: &'handler CallerScope,
    pub scope: &'handler McpRequestScope,
    pub ports: &'handler dyn SearchPorts,
    pub operations: &'handler dyn McpOperations,
    pub settings: &'handler dyn SearchSettings,
}

#[must_use]
pub fn json_string(value: &JsonValue) -> Option<&str> {
    match value {
        JsonValue::String(text) => Some(text),
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::Array(_)
        | JsonValue::Object(_) => None,
    }
}

impl ProxyToolIdentity {
    // the fields mcp_proxy_tool_id hashes; B4684 owns the hashing but no accessor, so the raw pair is read here
    pub fn from_tool(tool: &MCPTool) -> Result<Self, MCPProxyToolIdentityError> {
        let identity = tool
            .meta
            .as_ref()
            .and_then(|metadata| metadata.get(PROXY_IDENTITY_META_KEY))
            .ok_or(MCPProxyToolIdentityError::Missing)?;
        let JsonValue::Object(fields) = identity else {
            return Err(MCPProxyToolIdentityError::Invalid);
        };
        let server_id = fields
            .get("server_id")
            .and_then(json_string)
            .ok_or(MCPProxyToolIdentityError::InvalidField { field: "server_id" })?;
        let tool_name = fields
            .get("tool_name")
            .and_then(json_string)
            .ok_or(MCPProxyToolIdentityError::InvalidField { field: "tool_name" })?;
        Ok(Self {
            server_id: server_id.to_owned(),
            tool_name: tool_name.to_owned(),
        })
    }
}
