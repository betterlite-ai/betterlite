use std::convert::Infallible;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_mcp::request_headers::raw_headers::RawHeaders;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_semantic::{EmbedError, Embedder, SemanticTextIndex, Vector};
use betterlite_server::management_rbac::{FeatureAccessError, FeatureName};
use betterlite_server::mcp::search_handlers::ports::{
    RouterEmbedders, SearchPortError, SearchPorts, SearchSettings,
};
use betterlite_server::mcp::search_handlers::types::{
    AccessibleAgent, AccessibleSkill, AgentSearchOutcome, AgentSearchRequest, CallerScope,
    McpRequestScope, SkillSearchOutcome, SkillSearchRequest,
};
use betterlite_server::mcp::tool_search::settings::{
    MCPToolSearchSettings, MCPToolSearchSettingsError,
};
use secrecy::SecretString;
use serde_json::Value as JsonValue;
use serde_json::json;
use tokio::sync::Mutex as AsyncMutex;

use super::super::input::{Input, fixture_vec, invalid_input};
use super::records::{Records, allowed_call_json, caller_json, feature_name};

pub(crate) struct Fakes<'input> {
    pub(super) input: &'input Input,
    pub(super) seen: Arc<Mutex<Records>>,
    index: AsyncMutex<SemanticTextIndex>,
    pub(crate) caller: CallerScope,
    pub(crate) scope: McpRequestScope,
}

impl<'input> Fakes<'input> {
    pub(crate) fn new(input: &'input Input) -> Result<Self, Box<HelperError>> {
        Ok(Self {
            input,
            seen: Arc::new(Mutex::new(Records::default())),
            index: AsyncMutex::new(SemanticTextIndex::new(16)),
            caller: CallerScope {
                user_id: input.caller.user_id.clone(),
                team_id: input.caller.team_id.clone(),
                end_user_id: input.caller.end_user_id.clone(),
                api_key_hash: None,
                role: None,
            },
            scope: request_scope(input)?,
        })
    }

    pub(crate) fn observations(&self) -> JsonValue {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .value()
    }

    fn checked<T>(&self, caller: &CallerScope, items: Vec<T>) -> Result<Vec<T>, SearchPortError> {
        if caller != &self.caller {
            return Err(SearchPortError {
                detail: "the handler resolved its data for another caller".to_owned(),
            });
        }
        Ok(items)
    }

    fn record_skill_search(
        &self,
        query: &str,
        skills: &[AccessibleSkill],
        top_k: i64,
        embedding_model: Option<&str>,
    ) {
        let record = json!({
            "query": query,
            "top_k": top_k,
            "embedding_model": embedding_model,
            "skill_ids": skills.iter().map(|skill| skill.skill_id.as_str()).collect::<Vec<_>>(),
            "caller": caller_json(&self.caller),
        });
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .skill_searches
            .push(record);
    }

    pub(super) fn allowed_response(
        &self,
        caller: &CallerScope,
        scope: &McpRequestScope,
    ) -> Vec<String> {
        let record = allowed_call_json(caller, scope);
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .allowed_calls
            .push(record);
        self.input.allowed_servers.clone()
    }
}

fn request_scope(input: &Input) -> Result<McpRequestScope, Box<HelperError>> {
    let pairs: Vec<(Vec<u8>, Vec<u8>)> = input
        .raw_headers
        .iter()
        .map(|(name, value)| (name.as_bytes().to_vec(), value.as_bytes().to_vec()))
        .collect();
    let borrowed: Vec<(&[u8], &[u8])> = pairs
        .iter()
        .map(|(name, value)| (name.as_slice(), value.as_slice()))
        .collect();
    let raw_headers = RawHeaders::from_pairs(&borrowed)
        .map_err(|error| invalid_input(format!("raw_headers: {error}")))?;
    Ok(McpRequestScope {
        client_ip: input.client_ip.clone(),
        mcp_servers: input.mcp_servers.clone(),
        mcp_auth_header: input.mcp_auth_header.clone().map(SecretString::from),
        mcp_server_auth_headers: input
            .mcp_server_auth_headers
            .iter()
            .map(|(server, fields)| {
                let exposed = fields
                    .iter()
                    .map(|(name, value)| (name.clone(), SecretString::from(value.clone())))
                    .collect();
                (server.clone(), exposed)
            })
            .collect(),
        oauth2_headers: input
            .oauth2_headers
            .iter()
            .map(|(name, value)| (name.clone(), SecretString::from(value.clone())))
            .collect(),
        raw_headers,
    })
}

#[async_trait]
impl SearchPorts for Fakes<'_> {
    async fn check_feature_access_for_user(
        &self,
        caller: &CallerScope,
        feature: FeatureName,
    ) -> Result<(), FeatureAccessError<Infallible>> {
        let record = json!({"feature": feature_name(feature), "caller": caller_json(caller)});
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .feature_checks
            .push(record);
        if self.input.feature_denied {
            return Err(FeatureAccessError::Disabled(feature));
        }
        Ok(())
    }

    async fn accessible_agents(
        &self,
        caller: &CallerScope,
    ) -> Result<Vec<AccessibleAgent>, SearchPortError> {
        self.checked(caller, fixture_vec(&self.input.agents))
    }

    async fn search_agents(
        &self,
        request: AgentSearchRequest<'_>,
        agents: &[AccessibleAgent],
        embedding_model: Option<&str>,
        caller: &CallerScope,
    ) -> Result<AgentSearchOutcome, SearchPortError> {
        let agent_ids: Vec<&str> = agents.iter().map(|agent| agent.agent_id.as_str()).collect();
        let record = json!({
            "query": request.query,
            "top_k": request.top_k,
            "embedding_model": embedding_model,
            "agent_ids": agent_ids,
            "caller": caller_json(caller),
        });
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .agent_searches
            .push(record);
        Ok(self.input.agent_outcome())
    }

    async fn accessible_skills(
        &self,
        caller: &CallerScope,
    ) -> Result<Vec<AccessibleSkill>, SearchPortError> {
        self.checked(caller, fixture_vec(&self.input.skills))
    }

    async fn search_skills(
        &self,
        request: SkillSearchRequest<'_>,
        skills: &[AccessibleSkill],
        embedding_model: Option<&str>,
        _caller: &CallerScope,
    ) -> Result<SkillSearchOutcome, SearchPortError> {
        self.record_skill_search(request.query, skills, request.top_k, embedding_model);
        Ok(self.input.skill_outcome())
    }
}

impl SearchSettings for Fakes<'_> {
    fn agent_embedding_model(&self) -> Option<&str> {
        self.input.agent_embedding_model.as_deref()
    }

    fn skill_embedding_model(&self) -> Option<&str> {
        self.input.skill_embedding_model.as_deref()
    }

    fn mcp_tool_search_settings(
        &self,
    ) -> Result<MCPToolSearchSettings, MCPToolSearchSettingsError> {
        let settings = self.input.settings.clone().unwrap_or_default();
        serde_json::from_value(settings.clone()).map_err(|_error| settings_error(&settings))
    }

    fn llm_router(&self) -> Option<&dyn RouterEmbedders> {
        self.input.has_router.then_some(self)
    }

    fn mcp_search_index(&self) -> &AsyncMutex<SemanticTextIndex> {
        &self.index
    }
}

impl RouterEmbedders for Fakes<'_> {
    fn embedder(&self, embedding_model: &str) -> Box<dyn Embedder> {
        Box::new(RecordingEmbedder {
            seen: Arc::clone(&self.seen),
            model: embedding_model.to_owned(),
        })
    }
}

// serde folds MCPToolSearchSettingsError into its own message, so the failing range check is read back from the raw fields
fn settings_error(settings: &JsonValue) -> MCPToolSearchSettingsError {
    if settings
        .get("top_k")
        .and_then(JsonValue::as_i64)
        .is_some_and(|top_k| !(1..=100).contains(&top_k))
    {
        return MCPToolSearchSettingsError::TopKOutOfRange;
    }
    MCPToolSearchSettingsError::SimilarityThresholdOutOfRange
}

struct RecordingEmbedder {
    seen: Arc<Mutex<Records>>,
    model: String,
}

#[async_trait]
impl Embedder for RecordingEmbedder {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vector>, EmbedError> {
        let record = json!({"model": self.model, "texts": texts});
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .embeddings
            .push(record);
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
