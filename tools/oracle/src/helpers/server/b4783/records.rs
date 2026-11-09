use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_core::agents::AgentResponse;
use betterlite_semantic::{EmbedError, Embedder, Vector};
use betterlite_server::agent_endpoints::search::AgentEmbedders;
use betterlite_server::auth::auth_identity::AuthContext;
use serde::Deserialize;

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
    pub(crate) agent: AgentResponse,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResultInput {
    pub(crate) agent: AgentResponse,
    pub(crate) score: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RankInput {
    pub(crate) query: String,
    pub(crate) agents: Vec<AgentResponse>,
    pub(crate) top_k: usize,
    pub(crate) embedding_model: String,
    pub(crate) vectors: BTreeMap<String, Vector>,
    #[serde(default)]
    pub(crate) failure: Option<Failure>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SearchAgentsInput {
    pub(crate) query: String,
    pub(crate) agents: Vec<AgentResponse>,
    pub(crate) top_k: usize,
    pub(crate) embedding_model: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FailuresInput {
    pub(crate) query: String,
    pub(crate) agents: Vec<AgentResponse>,
    pub(crate) top_k: usize,
    pub(crate) embedding_model: String,
    pub(crate) message: String,
    pub(crate) http_status: u16,
    pub(crate) http_detail: String,
}

#[derive(Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Failure {
    Value { message: String },
    Http { status_code: u16, detail: String },
}

pub(crate) struct MapEmbedder {
    pub(crate) vectors: BTreeMap<String, Vector>,
    pub(crate) failure: Option<Failure>,
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

pub(crate) struct MissingEmbedders;

impl AgentEmbedders for MissingEmbedders {
    fn embedder_for(
        &self,
        _embedding_model: &str,
        _caller: &AuthContext,
    ) -> Option<Box<dyn Embedder>> {
        None
    }
}

pub(crate) struct EmbeddingProbe<'search> {
    pub(crate) query: &'search str,
    pub(crate) agents: &'search [AgentResponse],
    pub(crate) top_k: usize,
    pub(crate) embedder: &'search MapEmbedder,
    pub(crate) embedding_model: &'search str,
}
