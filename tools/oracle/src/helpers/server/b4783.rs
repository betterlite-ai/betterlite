mod records;

use std::collections::BTreeMap;
use std::fmt::Display;

use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType};
use betterlite_server::agent_endpoints::search::index::{AgentSearchEmbedding, AgentSearchIndex};
use betterlite_server::agent_endpoints::search::{
    AgentSearchError, AgentSearchHit, AgentSearchOutcome, AgentSearchRuntime, agent_search_result,
    agent_search_text, search_agents,
};
use betterlite_server::auth::auth_identity::AuthContext;
use serde_json::{Value, json};
use tokio::runtime::Runtime;

use self::records::{
    EmbeddingProbe, Failure, FailuresInput, MapEmbedder, MissingEmbedders, RankInput, ResultInput,
    SearchAgentsInput,
};
use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

impl Helper for Op {
    type Input = records::Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            records::Input::SearchText(call) => Ok(json!(agent_search_text(&call.agent))),
            records::Input::Result(call) => project(call),
            records::Input::Rank(call) => rank(call),
            records::Input::SearchAgents(call) => search(call),
            records::Input::Failures(call) => failures(call),
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
    let failed = Failure::Value {
        message: call.message.clone(),
    };
    let outcome = probe_outcome(&executor, &index, call, failed).map_err(route_error)?;
    let rejected = Failure::Http {
        status_code: call.http_status,
        detail: call.http_detail.clone(),
    };
    let http_error = match probe_outcome(&executor, &index, call, rejected) {
        Err(AgentSearchError::Http {
            status_code,
            detail,
        }) => json!({"status_code": status_code, "detail": detail}),
        Ok(_) => Value::Null,
    };
    Ok(json!({
        "embedding_failed": outcome_value(outcome),
        "http_error": http_error,
    }))
}

fn probe_outcome(
    executor: &Runtime,
    index: &AgentSearchIndex,
    call: &FailuresInput,
    failure: Failure,
) -> Result<AgentSearchOutcome, AgentSearchError> {
    let embedder = MapEmbedder {
        vectors: BTreeMap::new(),
        failure: Some(failure),
    };
    let probe = EmbeddingProbe {
        query: &call.query,
        agents: &call.agents,
        top_k: call.top_k,
        embedder: &embedder,
        embedding_model: &call.embedding_model,
    };
    probe_agents(executor, index, &probe)
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
