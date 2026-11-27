use std::collections::BTreeMap;

use betterlite_core::skills::search::SkillSearchResult;
use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_server::agent_endpoints::search::{AgentSearchResult, SearchableSkill};
use betterlite_server::mcp::catalog_guard::types::MCPTool;
use betterlite_server::mcp::hook_payloads::GuardrailContext;
use betterlite_server::mcp::search_handlers::types::{
    AccessibleAgent, AccessibleSkill, AgentSearchOutcome, SkillSearchOutcome,
};
use betterlite_server::mcp::tool_search::identity::with_mcp_proxy_identity;
use serde::Deserialize;
use serde_json::Value as Json;

const DEFAULT_TOP_K: i64 = 5;
const AGENT_SCORE: f64 = 0.5;
const SKILL_SCORE: f64 = 0.25;

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

impl Input {
    pub(super) fn query_text(&self) -> &str {
        self.query.as_deref().unwrap_or_default()
    }

    pub(super) fn top_k_value(&self) -> i64 {
        self.top_k.unwrap_or(DEFAULT_TOP_K)
    }

    pub(super) fn accessible_agents(&self) -> Vec<AccessibleAgent> {
        self.agents
            .iter()
            .map(|agent| AccessibleAgent {
                agent_id: agent.id.clone(),
                agent_name: agent.name.clone(),
            })
            .collect()
    }

    pub(super) fn accessible_skills(&self) -> Vec<AccessibleSkill> {
        self.skills
            .iter()
            .map(|skill| AccessibleSkill {
                skill_id: skill.id.clone(),
                display_title: Some(skill.name.clone()),
                description: skill.description.clone(),
            })
            .collect()
    }

    pub(super) fn agent_outcome(&self) -> AgentSearchOutcome {
        match self.agent_outcome {
            OutcomeKind::NotConfigured => AgentSearchOutcome::NotConfigured {
                reason: self.agent_reason.clone().unwrap_or_default(),
            },
            OutcomeKind::EmbeddingFailed => AgentSearchOutcome::EmbeddingFailed {
                reason: self.agent_reason.clone().unwrap_or_default(),
            },
            OutcomeKind::Hits => AgentSearchOutcome::Hits(
                self.agents
                    .iter()
                    .map(|agent| {
                        let id = &agent.id;
                        AgentSearchResult {
                            agent_id: id.clone(),
                            agent_name: agent.name.clone(),
                            description: format!("agent description of {id}"),
                            skills: vec![SearchableSkill {
                                name: format!("skill-{id}"),
                                description: format!("skill description of {id}"),
                                tags: vec!["seeded".to_owned()],
                            }],
                            score: AGENT_SCORE,
                        }
                    })
                    .collect(),
            ),
        }
    }

    pub(super) fn skill_outcome(&self) -> SkillSearchOutcome {
        match self.skill_outcome {
            OutcomeKind::NotConfigured => SkillSearchOutcome::NotConfigured {
                reason: self.skill_reason.clone().unwrap_or_default(),
            },
            OutcomeKind::EmbeddingFailed => SkillSearchOutcome::EmbeddingFailed {
                reason: self.skill_reason.clone().unwrap_or_default(),
            },
            OutcomeKind::Hits => SkillSearchOutcome::Hits(
                self.skills
                    .iter()
                    .map(|skill| SkillSearchResult {
                        skill_id: skill.id.clone(),
                        display_title: Some(skill.name.clone()),
                        description: skill.description.clone(),
                        score: SKILL_SCORE,
                    })
                    .collect(),
            ),
        }
    }

    pub(super) fn listed_tools(&self) -> Vec<MCPTool> {
        self.tools
            .iter()
            .map(|tool| {
                let catalog_tool = MCPTool {
                    name: tool.name.clone(),
                    description: tool.description.clone(),
                    title: None,
                    input_schema: tool.input_schema.clone(),
                    output_schema: tool.output_schema.clone(),
                    execution: None,
                    annotations: None,
                    icons: None,
                    meta: None,
                };
                with_mcp_proxy_identity(&catalog_tool, &tool.server_id)
            })
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
