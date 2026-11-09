use std::collections::BTreeMap;
use std::fmt::Display;

use async_trait::async_trait;
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType};
use betterlite_core::agents::AgentResponse;
use betterlite_semantic::{EmbedError, Embedder, Vector};
use betterlite_server::agent_endpoints::search::index::{AgentSearchEmbedding, AgentSearchIndex};
use betterlite_server::agent_endpoints::search::{
    AgentEmbedders, AgentSearchError, AgentSearchHit, AgentSearchOutcome, AgentSearchRuntime,
    agent_search_result, agent_search_text, search_agents,
};
use betterlite_server::auth::auth_identity::AuthContext;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Runtime;

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    SearchText(SearchTextInput),
    Result(ResultInput),
    Rank(RankInput),
    SearchAgents(SearchAgentsInput),
    Failures(FailuresInput),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SearchTextInput {
    agent: AgentResponse,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResultInput {
    agent: AgentResponse,
    score: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RankInput {
    query: String,
    agents: Vec<AgentResponse>,
    top_k: usize,
    embedding_model: String,
    vectors: BTreeMap<String, Vector>,
    #[serde(default)]
    failure: Option<Failure>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SearchAgentsInput {
    query: String,
    agents: Vec<AgentResponse>,
    top_k: usize,
    embedding_model: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FailuresInput {
    query: String,
    agents: Vec<AgentResponse>,
    top_k: usize,
    embedding_model: String,
    message: String,
    http_status: u16,
    http_detail: String,
}

#[derive(Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Failure {
    Value { message: String },
    Http { status_code: u16, detail: String },
}

struct MapEmbedder {
    vectors: BTreeMap<String, Vector>,
    failure: Option<Failure>,
}

#[async_trait]
impl Embedder for MapEmbedder {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vector>, EmbedError> {
        if let Some(failure) = &self.failure {
            return Err(match failure {
                Failure::Value { message } => EmbedError::Other(message.clone()),
                Failure::Http {
                    status_code,
                    detail,
                } => EmbedError::Http {
                    status_code: *status_code,
                    detail: detail.clone(),
                },
            });
        }
        texts
            .iter()
            .map(|text| {
                self.vectors
                    .get(text)
                    .cloned()
                    .ok_or_else(|| EmbedError::Other(format!("no vector for {text}")))
            })
            .collect()
    }
}

struct MissingEmbedders;

impl AgentEmbedders for MissingEmbedders {
    fn embedder_for(
        &self,
        _embedding_model: &str,
        _caller: &AuthContext,
    ) -> Option<Box<dyn Embedder>> {
        None
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::SearchText(call) => Ok(json!(agent_search_text(&call.agent))),
            Input::Result(call) => project(call),
            Input::Rank(call) => rank(call),
            Input::SearchAgents(call) => search(call),
            Input::Failures(call) => failures(call),
        }
    }
}

fn project(call: &ResultInput) -> HelperResult {
    let hit = AgentSearchHit {
        agent: call.agent.clone(),
        score: call.score,
    };
    serde_json::to_value(agent_search_result(&hit)).map_err(helper_error)
}

fn rank(call: &RankInput) -> HelperResult {
    let executor = executor()?;
    let index = AgentSearchIndex::default();
    let embedder = MapEmbedder {
        vectors: call.vectors.clone(),
        failure: call.failure.clone(),
    };
    let probe = EmbeddingProbe {
        query: &call.query,
        agents: &call.agents,
        top_k: call.top_k,
        embedder: &embedder,
        embedding_model: &call.embedding_model,
    };
    let outcome = probe_agents(&executor, &index, &probe).map_err(route_error)?;
    Ok(outcome_value(outcome))
}

fn search(call: &SearchAgentsInput) -> HelperResult {
    let executor = executor()?;
    let index = AgentSearchIndex::default();
    let caller = caller();
    let embedders = MissingEmbedders;
    let model_missing = executor
        .block_on(search_agents(
            &call.query,
            &call.agents,
            call.top_k,
            None,
            AgentSearchRuntime {
                embedders: &embedders,
                index: &index,
                caller: &caller,
            },
        ))
        .map_err(route_error)?;
    let router_missing = executor
        .block_on(search_agents(
            &call.query,
            &call.agents,
            call.top_k,
            Some(&call.embedding_model),
            AgentSearchRuntime {
                embedders: &embedders,
                index: &index,
                caller: &caller,
            },
        ))
        .map_err(route_error)?;
    Ok(json!({
        "model_missing": outcome_value(model_missing),
        "router_missing": outcome_value(router_missing),
    }))
}

fn failures(call: &FailuresInput) -> HelperResult {
    let executor = executor()?;
    let index = AgentSearchIndex::default();
    let outcome = embedding_failure(&executor, &index, call)?;
    let http_error = embedding_http_error(&executor, &index, call);
    Ok(json!({
        "embedding_failed": outcome_value(outcome),
        "http_error": http_error,
    }))
}

fn embedding_failure(
    executor: &Runtime,
    index: &AgentSearchIndex,
    call: &FailuresInput,
) -> Result<AgentSearchOutcome, Box<HelperError>> {
    let embedder = MapEmbedder {
        vectors: BTreeMap::new(),
        failure: Some(Failure::Value {
            message: call.message.clone(),
        }),
    };
    let probe = EmbeddingProbe {
        query: &call.query,
        agents: &call.agents,
        top_k: call.top_k,
        embedder: &embedder,
        embedding_model: &call.embedding_model,
    };
    probe_agents(executor, index, &probe).map_err(route_error)
}

fn embedding_http_error(
    executor: &Runtime,
    index: &AgentSearchIndex,
    call: &FailuresInput,
) -> Value {
    let embedder = MapEmbedder {
        vectors: BTreeMap::new(),
        failure: Some(Failure::Http {
            status_code: call.http_status,
            detail: call.http_detail.clone(),
        }),
    };
    let probe = EmbeddingProbe {
        query: &call.query,
        agents: &call.agents,
        top_k: call.top_k,
        embedder: &embedder,
        embedding_model: &call.embedding_model,
    };
    match probe_agents(executor, index, &probe) {
        Err(AgentSearchError::Http {
            status_code,
            detail,
        }) => json!({"status_code": status_code, "detail": detail}),
        Ok(_) => Value::Null,
    }
}

struct EmbeddingProbe<'search> {
    query: &'search str,
    agents: &'search [AgentResponse],
    top_k: usize,
    embedder: &'search MapEmbedder,
    embedding_model: &'search str,
}

fn probe_agents(
    executor: &Runtime,
    index: &AgentSearchIndex,
    probe: &EmbeddingProbe<'_>,
) -> Result<AgentSearchOutcome, AgentSearchError> {
    let embedding = AgentSearchEmbedding {
        embedder: probe.embedder,
        embedding_model: probe.embedding_model,
    };
    executor.block_on(index.search(probe.query, probe.agents, probe.top_k, embedding))
}

fn outcome_value(outcome: AgentSearchOutcome) -> Value {
    match outcome {
        AgentSearchOutcome::Hits(hits) => json!({
            "hits": hits
                .iter()
                .map(|hit| json!({"agent_id": hit.agent.agent_id, "score": hit.score}))
                .collect::<Vec<Value>>(),
        }),
        AgentSearchOutcome::NotConfigured { reason } => {
            json!({"not_configured": {"reason": reason}})
        }
        AgentSearchOutcome::EmbeddingFailed { reason } => json!({"failed": {"reason": reason}}),
    }
}

fn executor() -> Result<Runtime, Box<HelperError>> {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(helper_error)
}

fn caller() -> AuthContext {
    AuthContext {
        principal: Principal {
            principal_type: PrincipalType::ServiceAccount,
            subject: "oracle".to_owned(),
            issuer: None,
            audience: Vec::new(),
            user: None,
            organization: None,
            teams: Vec::new(),
            project: None,
            end_user: None,
            roles: Vec::new(),
            scopes: Vec::new(),
            auth_method: AuthMethod::ApiKey,
            credential_ref: CredentialRef::default(),
            network: NetworkContext::default(),
            source_key: SourceKey(None),
        },
    }
}

fn helper_error<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn http_error(status_code: u16, detail: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "HTTPException".to_owned(),
        status: Some(status_code),
        error_type: None,
        param: None,
        code: None,
        message: detail,
    })
}

fn route_error(error: AgentSearchError) -> Box<HelperError> {
    match error {
        AgentSearchError::Http {
            status_code,
            detail,
        } => http_error(status_code, detail),
    }
}
