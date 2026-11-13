use std::str::FromStr as _;

use betterlite_core::agents::AgentResponse;
use rust_decimal::Decimal;
use serde::Serialize;
use thiserror::Error;

use super::AgentSearchError;
use super::AgentSearchRuntime;
use super::{AgentSearchHit, AgentSearchOutcome, search_agents};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentSearchErrorDetail {
    pub error: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentSearchErrorBody {
    pub detail: AgentSearchErrorDetail,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct AgentSearchHttpError {
    pub status_code: u16,
    pub error: String,
    pub message: String,
}

impl From<&AgentSearchHttpError> for AgentSearchErrorBody {
    fn from(http_error: &AgentSearchHttpError) -> Self {
        Self {
            detail: AgentSearchErrorDetail {
                error: http_error.error.clone(),
                message: http_error.message.clone(),
            },
        }
    }
}

#[derive(Debug, Error)]
pub enum AgentSearchRouteError {
    #[error("{0}")]
    Http(AgentSearchHttpError),
    #[error("the embedding request failed with HTTP {status_code}: {detail}")]
    Embedding { status_code: u16, detail: String },
}

pub async fn rank_agents_by_query(
    query: &str,
    agents: &[AgentResponse],
    top_k: usize,
    embedding_model: Option<&str>,
    runtime: AgentSearchRuntime<'_>,
) -> Result<Vec<AgentResponse>, AgentSearchRouteError> {
    let outcome = search_agents(query, agents, top_k, embedding_model, runtime)
        .await
        .map_err(|error| match error {
            AgentSearchError::Http {
                status_code,
                detail,
            } => AgentSearchRouteError::Embedding {
                status_code,
                detail,
            },
        })?;
    match outcome {
        AgentSearchOutcome::Hits(hits) => hits.into_iter().map(scored_agent).collect(),
        AgentSearchOutcome::NotConfigured { reason } => {
            Err(AgentSearchRouteError::Http(AgentSearchHttpError {
                status_code: 400,
                error: "agent_search_not_configured".to_owned(),
                message: reason,
            }))
        }
        AgentSearchOutcome::EmbeddingFailed { reason } => {
            Err(AgentSearchRouteError::Http(AgentSearchHttpError {
                status_code: 503,
                error: "agent_search_unavailable".to_owned(),
                message: reason,
            }))
        }
    }
}

fn scored_agent(hit: AgentSearchHit) -> Result<AgentResponse, AgentSearchRouteError> {
    let AgentSearchHit { mut agent, score } = hit;
    // LiteLLM's response fails to serialize a non-finite score and ends in a 500
    let decimal = Decimal::from_str(&score.to_string()).map_err(|_source| {
        AgentSearchRouteError::Embedding {
            status_code: 500,
            detail: format!("agent search produced a non-finite score {score}"),
        }
    })?;
    agent.search_score = Some(decimal);
    Ok(agent)
}
