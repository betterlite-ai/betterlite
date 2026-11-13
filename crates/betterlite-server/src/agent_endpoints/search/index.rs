use std::cmp::Ordering;

use betterlite_core::agents::AgentResponse;
use betterlite_semantic::{Embedder, SemanticError, SemanticTextIndex};
use tokio::sync::Mutex;

use super::{AgentSearchError, AgentSearchHit, AgentSearchOutcome, agent_search_text};

#[derive(Clone, Copy)]
pub struct AgentSearchEmbedding<'search> {
    pub embedder: &'search dyn Embedder,
    pub embedding_model: &'search str,
}

#[derive(Default)]
pub struct AgentSearchIndex {
    index: Mutex<SemanticTextIndex>,
}

impl AgentSearchIndex {
    pub async fn search(
        &self,
        query: &str,
        agents: &[AgentResponse],
        top_k: usize,
        embedding: AgentSearchEmbedding<'_>,
    ) -> Result<AgentSearchOutcome, AgentSearchError> {
        let texts: Vec<String> = agents.iter().map(agent_search_text).collect();
        let resolved = {
            let mut index = self.index.lock().await;
            index
                .scores(query, &texts, embedding.embedder, embedding.embedding_model)
                .await
        };
        let scores = match resolved {
            Ok(scores) => scores,
            Err(SemanticError::Failed(failed)) => {
                return Ok(AgentSearchOutcome::EmbeddingFailed {
                    reason: failed.reason,
                });
            }
            Err(SemanticError::Http {
                status_code,
                detail,
            }) => {
                return Err(AgentSearchError::Http {
                    status_code,
                    detail,
                });
            }
        };
        let mut hits: Vec<AgentSearchHit> = agents
            .iter()
            .cloned()
            .zip(scores)
            .map(|(agent, score)| AgentSearchHit { agent, score })
            .collect();
        hits.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(Ordering::Equal)
        });
        hits.truncate(top_k);
        Ok(AgentSearchOutcome::Hits(hits))
    }
}
