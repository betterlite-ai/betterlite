use std::collections::BTreeMap;

use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_server::mcp::catalog_guard::types::MCPTool;
use betterlite_server::mcp::hook_payloads::GuardrailContext;
use betterlite_server::mcp::search_handlers::types::{
    AgentSearchOutcome, SearchOutcome, SkillSearchOutcome,
};
use betterlite_server::mcp::tool_search::identity::with_mcp_proxy_identity;
use serde::Deserialize;
use serde_json::Value as Json;

mod accessible;
mod agent_hits;
mod skill_hits;
mod tools;

pub(super) const DEFAULT_TOP_K: i64 = 5;
pub(super) const AGENT_SCORE: f64 = 0.5;
pub(super) const SKILL_SCORE: f64 = 0.25;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Handler {
    AgentSearch,
    SkillSearch,
    McpToolSearch,
    McpProxyTool,
    McpToolCall,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum OutcomeKind {
    #[default]
    Hits,
    NotConfigured,
    EmbeddingFailed,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(super) handler: Handler,
    pub(super) query: Option<String>,
    pub(super) top_k: Option<i64>,
    pub(super) name: Option<String>,
    #[serde(default)]
    pub(super) arguments: BTreeMap<String, JsonValue>,
    pub(super) tool_name: Option<String>,
    #[serde(default)]
    pub(super) call_arguments: BTreeMap<String, JsonValue>,
    pub(super) requested_server_id: Option<String>,
    pub(super) guardrail_context: Option<GuardrailContext>,
    #[serde(default)]
    pub(super) caller: CallerInput,
    #[serde(default)]
    pub(super) feature_denied: bool,
    #[serde(default)]
    pub(super) agents: Vec<NamedInput>,
    #[serde(default)]
    pub(super) skills: Vec<NamedInput>,
    #[serde(default)]
    pub(super) agent_outcome: OutcomeKind,
    pub(super) agent_reason: Option<String>,
    #[serde(default)]
    pub(super) skill_outcome: OutcomeKind,
    pub(super) skill_reason: Option<String>,
    pub(super) agent_embedding_model: Option<String>,
    pub(super) skill_embedding_model: Option<String>,
    pub(super) settings: Option<Json>,
    #[serde(default)]
    pub(super) has_router: bool,
    #[serde(default)]
    pub(super) tools: Vec<ToolInput>,
    pub(super) mcp_servers: Option<Vec<String>>,
    #[serde(default)]
    pub(super) allowed_servers: Vec<String>,
    pub(super) denial_detail: Option<String>,
    pub(super) client_ip: Option<String>,
    pub(super) mcp_auth_header: Option<String>,
    #[serde(default)]
    pub(super) mcp_server_auth_headers: BTreeMap<String, BTreeMap<String, String>>,
    #[serde(default)]
    pub(super) oauth2_headers: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) raw_headers: BTreeMap<String, String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CallerInput {
    pub(super) user_id: Option<String>,
    pub(super) team_id: Option<String>,
    pub(super) end_user_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NamedInput {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) description: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ToolInput {
    pub(super) name: String,
    pub(super) server_id: String,
    pub(super) description: Option<String>,
    #[serde(alias = "inputSchema", default)]
    pub(super) input_schema: BTreeMap<String, JsonValue>,
    #[serde(alias = "outputSchema", default)]
    pub(super) output_schema: Option<BTreeMap<String, JsonValue>>,
}

pub(super) trait FromFixture<T> {
    fn from_fixture(input: &T) -> Self;
}

pub(super) fn fixture_vec<T, S>(items: &[T]) -> Vec<S>
where
    S: FromFixture<T>,
{
    let mut fixtures = Vec::with_capacity(items.len());
    for item in items {
        fixtures.push(S::from_fixture(item));
    }
    fixtures
}

fn search_outcome<T>(kind: OutcomeKind, reason: String, hits: Vec<T>) -> SearchOutcome<T> {
    match kind {
        OutcomeKind::Hits => SearchOutcome::Hits(hits),
        OutcomeKind::NotConfigured => SearchOutcome::NotConfigured { reason },
        OutcomeKind::EmbeddingFailed => SearchOutcome::EmbeddingFailed { reason },
    }
}

impl Input {
    pub(super) fn agent_outcome(&self) -> AgentSearchOutcome {
        search_outcome(
            self.agent_outcome,
            self.agent_reason.clone().unwrap_or_default(),
            fixture_vec(&self.agents),
        )
    }

    pub(super) fn skill_outcome(&self) -> SkillSearchOutcome {
        search_outcome(
            self.skill_outcome,
            self.skill_reason.clone().unwrap_or_default(),
            fixture_vec(&self.skills),
        )
    }

    pub(super) fn listed_tools(&self) -> Vec<MCPTool> {
        self.tools
            .iter()
            .map(|tool| with_mcp_proxy_identity(&MCPTool::from_fixture(tool), &tool.server_id))
            .collect()
    }
}

pub(super) fn invalid_input(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
