use std::collections::BTreeMap;
use std::convert::Infallible;
use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_auth::user_roles::UserRole;
use betterlite_core::skills::search::SkillSearchResult;
use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_mcp::client::tools::result::{CallToolResult, ContentBlock};
use betterlite_mcp::wire_compat::tool_outcome::{TextResult, ToolOutcome};
use betterlite_semantic::{EmbedError, Embedder, SemanticTextIndex, Vector};
use rstest::rstest;
use tokio::sync::Mutex as AsyncMutex;

use super::error::SearchHandlerError;
use super::ports::{
    McpDenial, McpOperations, RouterEmbedders, SearchPortError, SearchPorts, SearchSettings,
    ToolExecution,
};
use super::types::{
    AccessibleAgent, AccessibleSkill, AgentSearchOutcome, AgentSearchRequest, CallerScope,
    HandlerContext, McpRequestScope, McpToolSearchRequest, ProxyToolRequest, SkillSearchOutcome,
    SkillSearchRequest, ToolCallRequest,
};
use super::{
    handle_agent_search, handle_mcp_proxy_tool, handle_mcp_tool_call, handle_mcp_tool_search,
    handle_skill_search,
};
use crate::agent_endpoints::search::{AgentSearchResult, SearchableSkill};
use crate::management_rbac::{FeatureAccessError, FeatureName};
use crate::mcp::catalog_guard::types::MCPTool;
use crate::mcp::hook_payloads::GuardrailContext;
use crate::mcp::tool_search::identity::{mcp_proxy_tool_id, with_mcp_proxy_identity};
use crate::mcp::tool_search::settings::{MCPToolSearchSettings, MCPToolSearchSettingsError};

#[derive(Clone, Debug, PartialEq)]
struct SearchCall {
    query: String,
    top_k: i64,
    embedding_model: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
struct ExecutionRecord {
    tool_name: String,
    arguments: BTreeMap<String, JsonValue>,
    allowed_servers: Vec<String>,
    requested_server_id: Option<String>,
    guardrail_context: Option<GuardrailContext>,
}

#[derive(Default)]
struct Observations {
    feature_checks: Vec<FeatureName>,
    agents: Vec<String>,
    skills: Vec<String>,
    agent_searches: Vec<SearchCall>,
    skill_searches: Vec<SearchCall>,
    list_modes: Vec<bool>,
    allowed_calls: Vec<()>,
    denials: Vec<Vec<String>>,
    executions: Vec<ExecutionRecord>,
}

struct FakeRouter;

impl RouterEmbedders for FakeRouter {
    fn embedder(&self, _embedding_model: &str) -> Box<dyn Embedder> {
        Box::new(FakeEmbedder)
    }
}

struct FakeEmbedder;

#[async_trait]
impl Embedder for FakeEmbedder {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vector>, EmbedError> {
        Ok(texts
            .iter()
            .map(|text| {
                if text.contains("alpha") {
                    vec![1.0_f64, 0.0_f64]
                } else {
                    vec![0.0_f64, 1.0_f64]
                }
            })
            .collect())
    }
}

struct Fake {
    deny_feature: bool,
    agents: Vec<AccessibleAgent>,
    agent_outcome: AgentSearchOutcome,
    skills: Vec<AccessibleSkill>,
    skill_outcome: SkillSearchOutcome,
    tools: Vec<MCPTool>,
    allowed_servers: Vec<String>,
    denial_detail: String,
    outcome: ToolOutcome,
    settings: MCPToolSearchSettings,
    settings_valid: bool,
    agent_model: Option<String>,
    skill_model: Option<String>,
    router: Option<Box<dyn RouterEmbedders>>,
    index: AsyncMutex<SemanticTextIndex>,
    seen: Mutex<Observations>,
}

impl Default for Fake {
    fn default() -> Self {
        Self {
            deny_feature: false,
            agents: Vec::new(),
            agent_outcome: AgentSearchOutcome::Hits(Vec::new()),
            skills: Vec::new(),
            skill_outcome: SkillSearchOutcome::Hits(Vec::new()),
            tools: Vec::new(),
            allowed_servers: Vec::new(),
            denial_detail: String::new(),
            outcome: ToolOutcome::Text(TextResult {
                text: "done".to_owned(),
            }),
            settings: MCPToolSearchSettings::default(),
            settings_valid: true,
            agent_model: None,
            skill_model: None,
            router: None,
            index: AsyncMutex::new(SemanticTextIndex::new(16)),
            seen: Mutex::new(Observations::default()),
        }
    }
}

#[async_trait]
impl SearchPorts for Fake {
    async fn check_feature_access_for_user(
        &self,
        _caller: &CallerScope,
        feature: FeatureName,
    ) -> Result<(), FeatureAccessError<Infallible>> {
        self.seen.lock().unwrap().feature_checks.push(feature);
        if self.deny_feature {
            return Err(FeatureAccessError::Disabled(feature));
        }
        Ok(())
    }

    async fn accessible_agents(
        &self,
        _caller: &CallerScope,
    ) -> Result<Vec<AccessibleAgent>, SearchPortError> {
        Ok(self.agents.clone())
    }

    async fn search_agents(
        &self,
        query: &str,
        agents: &[AccessibleAgent],
        top_k: i64,
        embedding_model: Option<&str>,
        _caller: &CallerScope,
    ) -> Result<AgentSearchOutcome, SearchPortError> {
        let mut seen = self.seen.lock().unwrap();
        seen.agents = agents.iter().map(|agent| agent.agent_id.clone()).collect();
        seen.agent_searches.push(SearchCall {
            query: query.to_owned(),
            top_k,
            embedding_model: embedding_model.map(str::to_owned),
        });
        Ok(self.agent_outcome.clone())
    }

    async fn accessible_skills(
        &self,
        _caller: &CallerScope,
    ) -> Result<Vec<AccessibleSkill>, SearchPortError> {
        Ok(self.skills.clone())
    }

    async fn search_skills(
        &self,
        query: &str,
        skills: &[AccessibleSkill],
        top_k: i64,
        embedding_model: Option<&str>,
        _caller: &CallerScope,
    ) -> Result<SkillSearchOutcome, SearchPortError> {
        let mut seen = self.seen.lock().unwrap();
        seen.skills = skills.iter().map(|skill| skill.skill_id.clone()).collect();
        seen.skill_searches.push(SearchCall {
            query: query.to_owned(),
            top_k,
            embedding_model: embedding_model.map(str::to_owned),
        });
        Ok(self.skill_outcome.clone())
    }
}

#[async_trait]
impl McpOperations for Fake {
    async fn _list_mcp_tools(
        &self,
        _caller: &CallerScope,
        _scope: &McpRequestScope,
        proxy_mode: bool,
    ) -> Result<Vec<MCPTool>, SearchPortError> {
        self.seen.lock().unwrap().list_modes.push(proxy_mode);
        Ok(self.tools.clone())
    }

    async fn _get_allowed_mcp_servers(
        &self,
        _caller: &CallerScope,
        _scope: &McpRequestScope,
    ) -> Result<Vec<String>, SearchPortError> {
        self.seen.lock().unwrap().allowed_calls.push(());
        Ok(self.allowed_servers.clone())
    }

    async fn raise_denied_scoped_mcp_access(
        &self,
        _caller: &CallerScope,
        _scope: &McpRequestScope,
        requested_names: &[String],
    ) -> McpDenial {
        self.seen
            .lock()
            .unwrap()
            .denials
            .push(requested_names.to_vec());
        McpDenial {
            detail: self.denial_detail.clone(),
        }
    }

    async fn execute_mcp_tool(
        &self,
        call: ToolExecution<'_>,
    ) -> Result<ToolOutcome, SearchPortError> {
        self.seen.lock().unwrap().executions.push(ExecutionRecord {
            tool_name: call.tool_name.to_owned(),
            arguments: call.arguments.clone(),
            allowed_servers: call.allowed_servers.to_vec(),
            requested_server_id: call.requested_server_id.map(str::to_owned),
            guardrail_context: call.guardrail_context.cloned(),
        });
        Ok(self.outcome.clone())
    }
}

impl SearchSettings for Fake {
    fn agent_embedding_model(&self) -> Option<&str> {
        self.agent_model.as_deref()
    }

    fn skill_embedding_model(&self) -> Option<&str> {
        self.skill_model.as_deref()
    }

    fn mcp_tool_search_settings(
        &self,
    ) -> Result<MCPToolSearchSettings, MCPToolSearchSettingsError> {
        if self.settings_valid {
            return Ok(self.settings.clone());
        }
        Err(MCPToolSearchSettingsError::TopKOutOfRange)
    }

    fn llm_router(&self) -> Option<&dyn RouterEmbedders> {
        self.router.as_deref()
    }

    fn mcp_search_index(&self) -> &AsyncMutex<SemanticTextIndex> {
        &self.index
    }
}

fn caller() -> CallerScope {
    CallerScope {
        user_id: Some("u1".to_owned()),
        role: Some(UserRole::InternalUser),
        ..CallerScope::default()
    }
}

fn context<'a>(
    fake: &'a Fake,
    caller: &'a CallerScope,
    scope: &'a McpRequestScope,
) -> HandlerContext<'a> {
    HandlerContext {
        caller,
        scope,
        ports: fake,
        operations: fake,
        settings: fake,
    }
}

fn text_of(result: &CallToolResult) -> String {
    let Some(ContentBlock::Text(text)) = result.content.first() else {
        panic!("a handler returns one text block");
    };
    text.text.clone()
}

fn agent_result(agent_id: &str) -> AgentSearchResult {
    AgentSearchResult {
        agent_id: agent_id.to_owned(),
        agent_name: "agent".to_owned(),
        description: String::new(),
        skills: Vec::<SearchableSkill>::new(),
        score: 0.5_f64,
    }
}

fn skill_result(skill_id: &str) -> SkillSearchResult {
    SkillSearchResult {
        skill_id: skill_id.to_owned(),
        display_title: None,
        description: None,
        score: 0.5_f64,
    }
}

fn accessible_agent(agent_id: &str) -> AccessibleAgent {
    AccessibleAgent {
        agent_id: agent_id.to_owned(),
        agent_name: "agent".to_owned(),
    }
}

fn accessible_skill(skill_id: &str) -> AccessibleSkill {
    AccessibleSkill {
        skill_id: skill_id.to_owned(),
        display_title: None,
        description: None,
    }
}

fn tool(name: &str, description: &str, schema: &str, server_id: &str) -> MCPTool {
    let input_schema: BTreeMap<String, JsonValue> =
        serde_json::from_str(schema).expect("a schema object");
    let listed = MCPTool {
        name: name.to_owned(),
        description: Some(description.to_owned()),
        title: None,
        input_schema,
        output_schema: None,
        execution: None,
        annotations: None,
        icons: None,
        meta: None,
    };
    with_mcp_proxy_identity(&listed, server_id)
}

fn proxy_arguments(tool_id: &str) -> BTreeMap<String, JsonValue> {
    BTreeMap::from([("tool_id".to_owned(), JsonValue::String(tool_id.to_owned()))])
}

fn object_arguments(fields: &[(&str, JsonValue)]) -> BTreeMap<String, JsonValue> {
    fields
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

#[tokio::test]
async fn agent_search_checks_agent_feature_access() {
    let fake = Fake {
        deny_feature: true,
        agents: vec![accessible_agent("a1")],
        ..Fake::default()
    };
    let caller = caller();
    let scope = McpRequestScope::default();
    let request = AgentSearchRequest {
        query: "task",
        top_k: 3,
    };
    let error = handle_agent_search(request, &context(&fake, &caller, &scope))
        .await
        .expect_err("a denied feature fails the call");
    assert!(matches!(
        error,
        SearchHandlerError::Feature(FeatureAccessError::Disabled(FeatureName::Agents))
    ));
    let seen = fake.seen.lock().unwrap();
    assert_eq!(seen.feature_checks, vec![FeatureName::Agents]);
    assert!(seen.agent_searches.is_empty());
}

#[rstest]
#[case::zero(0, 1)]
#[case::negative(-4, 1)]
#[case::kept(3, 3)]
#[tokio::test]
async fn agent_search_clamps_top_k_to_one(#[case] top_k: i64, #[case] expected: i64) {
    let fake = Fake {
        agents: vec![accessible_agent("a1")],
        agent_outcome: AgentSearchOutcome::Hits(vec![agent_result("a1")]),
        ..Fake::default()
    };
    let caller = caller();
    let scope = McpRequestScope::default();
    let request = AgentSearchRequest {
        query: "task",
        top_k,
    };
    let result = handle_agent_search(request, &context(&fake, &caller, &scope))
        .await
        .expect("search succeeds");
    assert!(!result.is_error);
    assert!(text_of(&result).contains("a1"));
    let seen = fake.seen.lock().unwrap();
    assert_eq!(seen.agents, vec!["a1".to_owned()]);
    assert_eq!(
        seen.agent_searches.first().expect("one search").top_k,
        expected
    );
}

#[rstest]
#[case::not_configured(AgentSearchOutcome::NotConfigured { reason: "no model".to_owned() }, "no model")]
#[case::embedding_failed(AgentSearchOutcome::EmbeddingFailed { reason: "boom".to_owned() }, "boom")]
#[tokio::test]
async fn agent_search_returns_the_reason_as_error_text(
    #[case] outcome: AgentSearchOutcome,
    #[case] reason: &str,
) {
    let fake = Fake {
        agent_outcome: outcome,
        ..Fake::default()
    };
    let caller = caller();
    let scope = McpRequestScope::default();
    let request = AgentSearchRequest {
        query: "task",
        top_k: 2,
    };
    let result = handle_agent_search(request, &context(&fake, &caller, &scope))
        .await
        .expect("a scripted outcome is not a failure");
    assert!(result.is_error);
    assert_eq!(text_of(&result), reason);
}

#[rstest]
#[case::zero(0, 1)]
#[case::kept(7, 7)]
#[case::past_the_maximum(500, 100)]
#[tokio::test]
async fn skill_search_clamps_top_k_bounds(#[case] top_k: i64, #[case] expected: i64) {
    let fake = Fake {
        skills: vec![accessible_skill("s1")],
        skill_outcome: SkillSearchOutcome::Hits(vec![skill_result("s1")]),
        ..Fake::default()
    };
    let caller = caller();
    let scope = McpRequestScope::default();
    let request = SkillSearchRequest {
        query: "task",
        top_k,
    };
    let result = handle_skill_search(request, &context(&fake, &caller, &scope))
        .await
        .expect("search succeeds");
    assert!(!result.is_error);
    assert!(text_of(&result).contains("s1"));
    let seen = fake.seen.lock().unwrap();
    assert_eq!(seen.skills, vec!["s1".to_owned()]);
    assert_eq!(
        seen.skill_searches.first().expect("one search").top_k,
        expected
    );
}

#[rstest]
#[case::not_configured(SkillSearchOutcome::NotConfigured { reason: "no model".to_owned() }, "no model")]
#[case::embedding_failed(SkillSearchOutcome::EmbeddingFailed { reason: "boom".to_owned() }, "boom")]
#[tokio::test]
async fn skill_search_returns_the_reason_as_error_text(
    #[case] outcome: SkillSearchOutcome,
    #[case] reason: &str,
) {
    let fake = Fake {
        skill_outcome: outcome,
        ..Fake::default()
    };
    let caller = caller();
    let scope = McpRequestScope::default();
    let request = SkillSearchRequest {
        query: "task",
        top_k: 2,
    };
    let result = handle_skill_search(request, &context(&fake, &caller, &scope))
        .await
        .expect("a scripted outcome is not a failure");
    assert!(result.is_error);
    assert_eq!(text_of(&result), reason);
}

#[tokio::test]
async fn mcp_tool_search_validates_config() {
    let fake = Fake {
        settings_valid: false,
        ..Fake::default()
    };
    let caller = caller();
    let scope = McpRequestScope::default();
    let request = McpToolSearchRequest {
        query: "task",
        top_k: 3,
    };
    let result = handle_mcp_tool_search(request, &context(&fake, &caller, &scope))
        .await
        .expect("an invalid setting is a tool error");
    assert!(result.is_error);
    assert!(text_of(&result).starts_with("litellm_settings.mcp_tool_search is invalid:"));
    assert!(fake.seen.lock().unwrap().list_modes.is_empty());
}

#[tokio::test]
async fn mcp_tool_search_needs_a_router_for_its_embedding_model() {
    let fake = Fake {
        settings: MCPToolSearchSettings {
            embedding_model: Some("embed".to_owned()),
            ..MCPToolSearchSettings::default()
        },
        ..Fake::default()
    };
    let caller = caller();
    let scope = McpRequestScope::default();
    let request = McpToolSearchRequest {
        query: "task",
        top_k: 3,
    };
    let result = handle_mcp_tool_search(request, &context(&fake, &caller, &scope))
        .await
        .expect("a missing model_list is a tool error");
    assert!(result.is_error);
    assert_eq!(
        text_of(&result),
        "litellm_settings.mcp_tool_search.embedding_model needs a model_list so it can be called"
    );
}

#[tokio::test]
async fn mcp_tool_search_without_embedding_uses_keyword_scores() {
    let fake = Fake {
        tools: vec![
            tool("alpha-tool", "finds alpha", "{}", "srv"),
            tool("beta-tool", "finds beta", "{}", "srv"),
        ],
        ..Fake::default()
    };
    let caller = caller();
    let scope = McpRequestScope::default();
    let request = McpToolSearchRequest {
        query: "alpha",
        top_k: 3,
    };
    let result = handle_mcp_tool_search(request, &context(&fake, &caller, &scope))
        .await
        .expect("search succeeds");
    assert!(!result.is_error);
    let text = text_of(&result);
    assert!(text.contains("alpha-tool"));
    assert!(!text.contains("beta-tool"));
    assert_eq!(fake.seen.lock().unwrap().list_modes, vec![false]);
}

#[tokio::test]
async fn mcp_tool_search_ranks_semantically_when_configured() {
    let fake = Fake {
        tools: vec![
            tool("alpha-tool", "finds alpha", "{}", "srv"),
            tool("beta-tool", "finds beta", "{}", "srv"),
        ],
        settings: MCPToolSearchSettings {
            embedding_model: Some("embed".to_owned()),
            ..MCPToolSearchSettings::default()
        },
        router: Some(Box::new(FakeRouter)),
        ..Fake::default()
    };
    let caller = caller();
    let scope = McpRequestScope::default();
    let request = McpToolSearchRequest {
        query: "alpha",
        top_k: 3,
    };
    let result = handle_mcp_tool_search(request, &context(&fake, &caller, &scope))
        .await
        .expect("search succeeds");
    assert!(!result.is_error);
    let text = text_of(&result);
    assert!(text.contains("alpha-tool"));
    assert!(text.contains("beta-tool"));
    assert!(text.contains("\"score\""));
}

#[tokio::test]
async fn proxy_tool_search_returns_opaque_ids() {
    let tools: Vec<MCPTool> = (0..6)
        .map(|index| tool(&format!("tool-{index}"), "matches", "{}", "srv"))
        .collect();
    let fake = Fake {
        tools,
        settings: MCPToolSearchSettings {
            top_k: 100,
            ..MCPToolSearchSettings::default()
        },
        ..Fake::default()
    };
    let caller = caller();
    let scope = McpRequestScope::default();
    let arguments = object_arguments(&[("query", JsonValue::String("matches".to_owned()))]);
    let request = ProxyToolRequest {
        name: "search_tools",
        arguments: &arguments,
    };
    let result = handle_mcp_proxy_tool(request, &context(&fake, &caller, &scope))
        .await
        .expect("search succeeds");
    assert!(!result.is_error);
    let results: Vec<JsonValue> = serde_json::from_str(&text_of(&result)).expect("a JSON array");
    assert_eq!(results.len(), 5);
    assert_eq!(fake.seen.lock().unwrap().list_modes, vec![true]);
}

#[tokio::test]
async fn proxy_tool_resolves_opaque_tool_id() {
    let listed = tool("alpha", "finds alpha", "{}", "srv");
    let tool_id = mcp_proxy_tool_id(&listed).expect("an identified tool");
    let fake = Fake {
        tools: vec![listed],
        ..Fake::default()
    };
    let caller = caller();
    let scope = McpRequestScope::default();
    let arguments = proxy_arguments(&tool_id);
    let request = ProxyToolRequest {
        name: "get_tool_schema",
        arguments: &arguments,
    };
    let result = handle_mcp_proxy_tool(request, &context(&fake, &caller, &scope))
        .await
        .expect("a known id resolves");
    assert!(!result.is_error);
    assert!(text_of(&result).contains(&tool_id));
}

#[tokio::test]
async fn proxy_tool_rejects_unknown_id() {
    let fake = Fake {
        tools: vec![tool("alpha", "finds alpha", "{}", "srv")],
        ..Fake::default()
    };
    let caller = caller();
    let scope = McpRequestScope::default();
    let arguments = proxy_arguments("does-not-exist");
    let request = ProxyToolRequest {
        name: "get_tool_schema",
        arguments: &arguments,
    };
    let result = handle_mcp_proxy_tool(request, &context(&fake, &caller, &scope))
        .await
        .expect("an unknown id is a tool error");
    assert!(result.is_error);
    assert_eq!(text_of(&result), "Unknown or unauthorized tool_id");
}

#[tokio::test]
async fn proxy_tool_rejects_an_unknown_name() {
    let listed = tool("alpha", "finds alpha", "{}", "srv");
    let tool_id = mcp_proxy_tool_id(&listed).expect("an identified tool");
    let fake = Fake {
        tools: vec![listed],
        ..Fake::default()
    };
    let caller = caller();
    let scope = McpRequestScope::default();
    let arguments = proxy_arguments(&tool_id);
    let request = ProxyToolRequest {
        name: "not_a_proxy_tool",
        arguments: &arguments,
    };
    let error = handle_mcp_proxy_tool(request, &context(&fake, &caller, &scope))
        .await
        .expect_err("an unknown proxy name fails the call");
    assert_eq!(error.status_code(), http::StatusCode::BAD_REQUEST);
    assert_eq!(error.detail(), "Unknown MCP proxy tool: not_a_proxy_tool");
}

const REQUIRED_SCHEMA: &str =
    r#"{"type":"object","properties":{"x":{"type":"string"}},"required":["x"]}"#;

#[tokio::test]
async fn proxy_tool_call_validates_arguments() {
    let listed = tool("alpha", "finds alpha", REQUIRED_SCHEMA, "srv");
    let tool_id = mcp_proxy_tool_id(&listed).expect("an identified tool");
    let fake = Fake {
        tools: vec![listed],
        allowed_servers: vec!["srv".to_owned()],
        ..Fake::default()
    };
    let caller = caller();
    let scope = McpRequestScope::default();
    let arguments = object_arguments(&[
        ("tool_id", JsonValue::String(tool_id)),
        ("arguments", JsonValue::Object(BTreeMap::new())),
    ]);
    let request = ProxyToolRequest {
        name: "call_tool",
        arguments: &arguments,
    };
    let result = handle_mcp_proxy_tool(request, &context(&fake, &caller, &scope))
        .await
        .expect("invalid arguments are a tool error");
    assert!(result.is_error);
    assert!(text_of(&result).starts_with("Invalid arguments:"));
    assert!(fake.seen.lock().unwrap().executions.is_empty());
}

#[tokio::test]
async fn proxy_tool_call_delegates_to_the_tool_call_handler() {
    let listed = tool("alpha", "finds alpha", REQUIRED_SCHEMA, "srv");
    let tool_id = mcp_proxy_tool_id(&listed).expect("an identified tool");
    let fake = Fake {
        tools: vec![listed],
        allowed_servers: vec!["srv".to_owned()],
        ..Fake::default()
    };
    let caller = caller();
    let scope = McpRequestScope::default();
    let arguments = object_arguments(&[
        ("tool_id", JsonValue::String(tool_id)),
        (
            "arguments",
            JsonValue::Object(BTreeMap::from([(
                "x".to_owned(),
                JsonValue::String("value".to_owned()),
            )])),
        ),
    ]);
    let request = ProxyToolRequest {
        name: "call_tool",
        arguments: &arguments,
    };
    let result = handle_mcp_proxy_tool(request, &context(&fake, &caller, &scope))
        .await
        .expect("a valid call executes");
    assert_eq!(text_of(&result), "done");
    let seen = fake.seen.lock().unwrap();
    let execution = seen.executions.first().expect("one execution");
    assert_eq!(execution.tool_name, "alpha");
    assert_eq!(execution.requested_server_id.as_deref(), Some("srv"));
    assert_eq!(execution.allowed_servers, vec!["srv".to_owned()]);
    assert!(execution.guardrail_context.is_none());
    assert!(execution.arguments.contains_key("x"));
}

#[tokio::test]
async fn proxy_tool_search_validates_config() {
    let fake = Fake {
        settings_valid: false,
        ..Fake::default()
    };
    let caller = caller();
    let scope = McpRequestScope::default();
    let arguments = object_arguments(&[("query", JsonValue::String("task".to_owned()))]);
    let request = ProxyToolRequest {
        name: "search_tools",
        arguments: &arguments,
    };
    let result = handle_mcp_proxy_tool(request, &context(&fake, &caller, &scope))
        .await
        .expect("an invalid setting is a tool error");
    assert!(result.is_error);
    assert_eq!(
        text_of(&result),
        "mcp_tool_search.top_k must be between 1 and 100"
    );
}

#[tokio::test]
async fn proxy_tool_search_needs_a_router_for_its_embedding_model() {
    let fake = Fake {
        settings: MCPToolSearchSettings {
            embedding_model: Some("embed".to_owned()),
            ..MCPToolSearchSettings::default()
        },
        ..Fake::default()
    };
    let caller = caller();
    let scope = McpRequestScope::default();
    let arguments = object_arguments(&[("query", JsonValue::String("task".to_owned()))]);
    let request = ProxyToolRequest {
        name: "search_tools",
        arguments: &arguments,
    };
    let result = handle_mcp_proxy_tool(request, &context(&fake, &caller, &scope))
        .await
        .expect("a missing model_list is a tool error");
    assert!(result.is_error);
    assert_eq!(
        text_of(&result),
        "litellm_settings.mcp_tool_search.embedding_model needs a model_list so it can be called"
    );
}

#[tokio::test]
async fn mcp_tool_call_checks_authorization() {
    let fake = Fake::default();
    let caller = caller();
    let scope = McpRequestScope::default();
    let arguments = BTreeMap::new();
    let request = ToolCallRequest {
        tool_name: "alpha",
        arguments: &arguments,
        requested_server_id: None,
        guardrail_context: None,
    };
    let error = handle_mcp_tool_call(request, &context(&fake, &caller, &scope))
        .await
        .expect_err("no allowed server fails the call");
    assert_eq!(error.status_code(), http::StatusCode::FORBIDDEN);
    assert_eq!(error.detail(), "User not allowed to call this tool.");
    assert!(fake.seen.lock().unwrap().executions.is_empty());
}

#[tokio::test]
async fn mcp_tool_call_denies_explicit_scope() {
    let fake = Fake {
        denial_detail: "denied srv".to_owned(),
        ..Fake::default()
    };
    let caller = caller();
    let scope = McpRequestScope {
        mcp_servers: Some(vec!["srv".to_owned()]),
        ..McpRequestScope::default()
    };
    let arguments = BTreeMap::new();
    let request = ToolCallRequest {
        tool_name: "alpha",
        arguments: &arguments,
        requested_server_id: None,
        guardrail_context: None,
    };
    let error = handle_mcp_tool_call(request, &context(&fake, &caller, &scope))
        .await
        .expect_err("an empty scoped listing fails the call");
    assert_eq!(error.status_code(), http::StatusCode::FORBIDDEN);
    assert_eq!(error.detail(), "denied srv");
    assert_eq!(
        fake.seen.lock().unwrap().denials,
        vec![vec!["srv".to_owned()]]
    );
}

#[tokio::test]
async fn mcp_tool_call_preserves_guardrail_context() {
    let fake = Fake {
        allowed_servers: vec!["srv".to_owned()],
        ..Fake::default()
    };
    let caller = caller();
    let scope = McpRequestScope {
        client_ip: Some("10.0.0.1".to_owned()),
        ..McpRequestScope::default()
    };
    let guardrail = GuardrailContext {
        model: Some("gpt".to_owned()),
        ..GuardrailContext::default()
    };
    let arguments = object_arguments(&[("x", JsonValue::String("value".to_owned()))]);
    let request = ToolCallRequest {
        tool_name: "alpha",
        arguments: &arguments,
        requested_server_id: Some("srv"),
        guardrail_context: Some(&guardrail),
    };
    let result = handle_mcp_tool_call(request, &context(&fake, &caller, &scope))
        .await
        .expect("an allowed call executes");
    assert_eq!(text_of(&result), "done");
    let seen = fake.seen.lock().unwrap();
    assert_eq!(seen.allowed_calls.len(), 1);
    let execution = seen.executions.first().expect("one execution");
    assert_eq!(execution.tool_name, "alpha");
    assert_eq!(execution.requested_server_id.as_deref(), Some("srv"));
    assert_eq!(
        execution
            .guardrail_context
            .as_ref()
            .and_then(|value| value.model.as_deref()),
        Some("gpt")
    );
}
