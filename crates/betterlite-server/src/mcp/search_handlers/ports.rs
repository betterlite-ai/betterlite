use std::collections::BTreeMap;
use std::convert::Infallible;

use async_trait::async_trait;
use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_mcp::wire_compat::tool_outcome::ToolOutcome;
use betterlite_semantic::{Embedder, SemanticTextIndex};
use thiserror::Error;
use tokio::sync::Mutex;

use crate::management_rbac::{FeatureAccessError, FeatureName};
use crate::mcp::catalog_guard::types::MCPTool;
use crate::mcp::hook_payloads::GuardrailContext;
use crate::mcp::tool_search::settings::{MCPToolSearchSettings, MCPToolSearchSettingsError};

use super::types::{
    AccessibleAgent, AccessibleSkill, AgentSearchOutcome, CallerScope, McpRequestScope,
    SkillSearchOutcome,
};

#[derive(Debug, Error)]
#[error("{detail}")]
pub struct SearchPortError {
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpDenial {
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolExecution<'a> {
    pub scope: &'a McpRequestScope,
    pub caller: &'a CallerScope,
    pub tool_name: &'a str,
    pub arguments: &'a BTreeMap<String, JsonValue>,
    pub allowed_servers: &'a [String],
    pub requested_server_id: Option<&'a str>,
    pub guardrail_context: Option<&'a GuardrailContext>,
}

// the agent, skill and RBAC layers a handler calls; every method is another task's implementation
#[async_trait]
pub trait SearchPorts: Send + Sync {
    async fn check_feature_access_for_user(
        &self,
        caller: &CallerScope,
        feature: FeatureName,
    ) -> Result<(), FeatureAccessError<Infallible>>;

    async fn accessible_agents(
        &self,
        caller: &CallerScope,
    ) -> Result<Vec<AccessibleAgent>, SearchPortError>;

    async fn search_agents(
        &self,
        query: &str,
        agents: &[AccessibleAgent],
        top_k: i64,
        embedding_model: Option<&str>,
        caller: &CallerScope,
    ) -> Result<AgentSearchOutcome, SearchPortError>;

    async fn accessible_skills(
        &self,
        caller: &CallerScope,
    ) -> Result<Vec<AccessibleSkill>, SearchPortError>;

    async fn search_skills(
        &self,
        query: &str,
        skills: &[AccessibleSkill],
        top_k: i64,
        embedding_model: Option<&str>,
        caller: &CallerScope,
    ) -> Result<SkillSearchOutcome, SearchPortError>;
}

// LiteLLM's operations module: listing, the allowed-server scope, denial and execution
#[async_trait]
pub trait McpOperations: Send + Sync {
    async fn _list_mcp_tools(
        &self,
        caller: &CallerScope,
        scope: &McpRequestScope,
        proxy_mode: bool,
    ) -> Result<Vec<MCPTool>, SearchPortError>;

    async fn _get_allowed_mcp_servers(
        &self,
        caller: &CallerScope,
        scope: &McpRequestScope,
    ) -> Result<Vec<String>, SearchPortError>;

    async fn raise_denied_scoped_mcp_access(
        &self,
        caller: &CallerScope,
        scope: &McpRequestScope,
        requested_names: &[String],
    ) -> McpDenial;

    async fn execute_mcp_tool(
        &self,
        call: ToolExecution<'_>,
    ) -> Result<ToolOutcome, SearchPortError>;
}

pub trait RouterEmbedders: Send + Sync {
    fn embedder(&self, embedding_model: &str) -> Box<dyn Embedder>;
}

// the proxy settings the handlers read: both semantic-search models and litellm_settings.mcp_tool_search
pub trait SearchSettings: Send + Sync {
    fn agent_embedding_model(&self) -> Option<&str>;
    fn skill_embedding_model(&self) -> Option<&str>;
    fn mcp_tool_search_settings(&self)
    -> Result<MCPToolSearchSettings, MCPToolSearchSettingsError>;
    fn llm_router(&self) -> Option<&dyn RouterEmbedders>;
    fn mcp_search_index(&self) -> &Mutex<SemanticTextIndex>;
}
