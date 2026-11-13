use betterlite_core::agents::{AgentResponse, AgentSkill};
use betterlite_semantic::Embedder;
use serde::Serialize;
use thiserror::Error;

use crate::auth::auth_identity::AuthContext;

pub mod index;
pub mod rank;
mod tests;

use self::index::{AgentSearchEmbedding, AgentSearchIndex};

pub const DEFAULT_AGENT_SEARCH_TOP_K: usize = 5;

#[derive(Clone, Debug)]
pub struct AgentSearchHit {
    pub agent: AgentResponse,
    pub score: f64,
}

#[derive(Clone, Debug)]
pub enum AgentSearchOutcome {
    Hits(Vec<AgentSearchHit>),
    NotConfigured { reason: String },
    EmbeddingFailed { reason: String },
}

#[derive(Debug, Error)]
pub enum AgentSearchError {
    #[error("the embedding request failed with HTTP {status_code}: {detail}")]
    Http { status_code: u16, detail: String },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SearchableSkill {
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AgentSearchResult {
    pub agent_id: String,
    pub agent_name: String,
    pub description: String,
    pub skills: Vec<SearchableSkill>,
    pub score: f64,
}

#[must_use]
pub fn agent_search_text(agent: &AgentResponse) -> String {
    let card = &agent.agent_card_params;
    let mut parts: Vec<String> = Vec::new();
    if !agent.agent_name.is_empty() {
        parts.push(agent.agent_name.clone());
    }
    if let Some(description) = card.description.as_deref().filter(|text| !text.is_empty()) {
        parts.push(description.to_owned());
    }
    for skill in card.skills.iter().flatten() {
        let line = searchable_skill_text(skill);
        if !line.is_empty() {
            parts.push(line);
        }
    }
    parts.join("\n")
}

fn searchable_skill_text(skill: &AgentSkill) -> String {
    let name = skill.name.as_deref().unwrap_or_default();
    let description = skill.description.as_deref().unwrap_or_default();
    let tags = skill.tags.as_deref().unwrap_or_default().join(" ");
    [name, description, tags.as_str()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<&str>>()
        .join(" ")
}

#[must_use]
pub fn agent_search_result(hit: &AgentSearchHit) -> AgentSearchResult {
    let card = &hit.agent.agent_card_params;
    AgentSearchResult {
        agent_id: hit.agent.agent_id.clone(),
        agent_name: hit.agent.agent_name.clone(),
        description: card.description.clone().unwrap_or_default(),
        skills: card
            .skills
            .iter()
            .flatten()
            .map(|skill| SearchableSkill {
                name: skill.name.clone().unwrap_or_default(),
                description: skill.description.clone().unwrap_or_default(),
                tags: skill.tags.clone().unwrap_or_default(),
            })
            .collect(),
        score: hit.score,
    }
}

pub trait AgentEmbedders: Send + Sync {
    fn embedder_for(
        &self,
        embedding_model: &str,
        caller: &AuthContext,
    ) -> Option<Box<dyn Embedder>>;
}

#[derive(Clone, Copy)]
pub struct AgentSearchRuntime<'search> {
    pub embedders: &'search dyn AgentEmbedders,
    pub index: &'search AgentSearchIndex,
    pub caller: &'search AuthContext,
}

pub async fn search_agents(
    query: &str,
    agents: &[AgentResponse],
    top_k: usize,
    embedding_model: Option<&str>,
    runtime: AgentSearchRuntime<'_>,
) -> Result<AgentSearchOutcome, AgentSearchError> {
    let Some(model) = embedding_model else {
        return Ok(AgentSearchOutcome::NotConfigured {
            reason: "agent search needs litellm_settings.agent_search_embedding_model set to an embedding model from model_list".to_owned(),
        });
    };
    let Some(embedder) = runtime.embedders.embedder_for(model, runtime.caller) else {
        return Ok(AgentSearchOutcome::NotConfigured {
            reason: "agent search needs a model_list so the embedding model can be called"
                .to_owned(),
        });
    };
    let embedding = AgentSearchEmbedding {
        embedder: embedder.as_ref(),
        embedding_model: model,
    };
    runtime.index.search(query, agents, top_k, embedding).await
}
