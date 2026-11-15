use std::cmp::Ordering;

use betterlite_auth::auth_identity::records::Principal;
use betterlite_db::managed_resource_rows::skills::SkillTable;
use betterlite_semantic::{DEFAULT_MAX_CACHED_VECTORS, Embedder, SemanticError, SemanticTextIndex};
use serde::Serialize;
use thiserror::Error;
use tokio::sync::Mutex;

use crate::api::skills::get_delete::call::LITELLM_PROXY_PROVIDER;
use crate::skills::store::repository::SkillRepository;
use crate::skills::store::{HostedSkillStore, StoreError};

pub const DEFAULT_SKILL_SEARCH_TOP_K: usize = 5;
pub const MAX_SKILL_SEARCH_TOP_K: usize = 100;
pub const MAX_SKILL_SEARCH_TEXT_CHARS: usize = 4000;

#[derive(Clone, Debug, PartialEq)]
pub struct SkillSearchHit {
    pub skill: SkillTable,
    pub score: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SkillSearchResult {
    pub skill_id: String,
    pub display_title: Option<String>,
    pub description: Option<String>,
    pub score: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SkillSearchOutcome {
    Hits(Vec<SkillSearchHit>),
    NotConfigured { reason: String },
    EmbeddingFailed { reason: String },
    UnsupportedProvider { reason: String },
}

#[derive(Debug, Error)]
pub enum SkillSearchError {
    #[error("the embedding request failed with HTTP {status_code}: {detail}")]
    Http { status_code: u16, detail: String },
}

#[derive(Debug, Error)]
pub enum HostedSkillSearchError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Search(#[from] SkillSearchError),
}

#[derive(Clone, Copy)]
pub struct SkillSearchEmbedding<'search> {
    pub embedder: &'search dyn Embedder,
    pub embedding_model: &'search str,
}

#[derive(Clone, Copy)]
pub struct SkillSearchRuntime<'search> {
    // None is a proxy without a router, which LiteLLM reports apart from an unset embedding model
    pub embedder: Option<&'search dyn Embedder>,
    pub embedding_model: Option<&'search str>,
    pub index: &'search SkillSearchIndex,
}

#[derive(Clone, Copy)]
pub struct HostedSkillSearchRequest<'search> {
    pub custom_llm_provider: Option<&'search str>,
    pub query: &'search str,
    pub top_k: usize,
    pub runtime: SkillSearchRuntime<'search>,
}

pub struct SkillSearchIndex {
    index: Mutex<SemanticTextIndex>,
}

impl Default for SkillSearchIndex {
    // LiteLLM sizes the skill index by MAX_SKILLS_PER_SEARCH, the semantic index's default bound
    fn default() -> Self {
        Self::new(DEFAULT_MAX_CACHED_VECTORS)
    }
}

impl SkillSearchIndex {
    #[must_use]
    pub fn new(max_entries: usize) -> Self {
        Self {
            index: Mutex::new(SemanticTextIndex::new(max_entries)),
        }
    }

    pub async fn search(
        &self,
        query: &str,
        skills: &[SkillTable],
        top_k: usize,
        embedding: SkillSearchEmbedding<'_>,
    ) -> Result<SkillSearchOutcome, SkillSearchError> {
        let texts: Vec<String> = skills.iter().map(skill_search_text).collect();
        let resolved = {
            let mut index = self.index.lock().await;
            index
                .scores(query, &texts, embedding.embedder, embedding.embedding_model)
                .await
        };
        let scores = match resolved {
            Ok(scores) => scores,
            Err(SemanticError::Failed(failed)) => {
                return Ok(SkillSearchOutcome::EmbeddingFailed {
                    reason: failed.reason,
                });
            }
            Err(SemanticError::Http {
                status_code,
                detail,
            }) => {
                return Err(SkillSearchError::Http {
                    status_code,
                    detail,
                });
            }
        };
        match ranked_hits(skills, scores, top_k, embedding.embedding_model) {
            Ok(hits) => Ok(SkillSearchOutcome::Hits(hits)),
            Err(reason) => Ok(SkillSearchOutcome::EmbeddingFailed { reason }),
        }
    }
}

// LiteLLM sorts by score alone, so a tie keeps the row order the store returned
fn ranked_hits(
    skills: &[SkillTable],
    scores: Vec<f64>,
    top_k: usize,
    embedding_model: &str,
) -> Result<Vec<SkillSearchHit>, String> {
    let mut ranked: Vec<(&SkillTable, f64)> = skills.iter().zip(scores).collect();
    // Every score is checked before the sort and the slice, so a NaN is never ranked into an answer
    for (_, score) in &ranked {
        if !score.is_finite() {
            return Err(format!(
                "embedding model {embedding_model} returned a score that is not a finite number"
            ));
        }
    }
    ranked.sort_by(|left, right| right.1.partial_cmp(&left.1).unwrap_or(Ordering::Equal));
    ranked.truncate(top_k);
    Ok(ranked
        .into_iter()
        .map(|(skill, score)| SkillSearchHit {
            skill: skill.clone(),
            score,
        })
        .collect())
}

#[must_use]
pub fn skill_search_text(skill: &SkillTable) -> String {
    let joined = [
        skill.display_title.as_deref(),
        skill.description.as_deref(),
        skill.instructions.as_deref(),
    ]
    .into_iter()
    .flatten()
    .filter(|part| !part.is_empty())
    .collect::<Vec<&str>>()
    .join("\n");
    joined.chars().take(MAX_SKILL_SEARCH_TEXT_CHARS).collect()
}

#[must_use]
pub fn skill_search_result(hit: &SkillSearchHit) -> SkillSearchResult {
    SkillSearchResult {
        skill_id: hit.skill.skill_id.clone(),
        display_title: hit.skill.display_title.clone(),
        description: hit.skill.description.clone(),
        score: hit.score,
    }
}

pub async fn search_skills(
    query: &str,
    skills: &[SkillTable],
    top_k: usize,
    runtime: SkillSearchRuntime<'_>,
) -> Result<SkillSearchOutcome, SkillSearchError> {
    let Some(embedding_model) = runtime.embedding_model else {
        return Ok(SkillSearchOutcome::NotConfigured {
            reason: "skill search needs litellm_settings.skill_search_embedding_model set to an embedding model from model_list".to_owned(),
        });
    };
    let Some(embedder) = runtime.embedder else {
        return Ok(SkillSearchOutcome::NotConfigured {
            reason: "skill search needs a model_list so the embedding model can be called"
                .to_owned(),
        });
    };
    runtime
        .index
        .search(
            query,
            skills,
            top_k,
            SkillSearchEmbedding {
                embedder,
                embedding_model,
            },
        )
        .await
}

pub async fn search_hosted_skills<R: SkillRepository>(
    store: &HostedSkillStore<R>,
    principal: Option<&Principal>,
    search: HostedSkillSearchRequest<'_>,
) -> Result<SkillSearchOutcome, HostedSkillSearchError> {
    if search.custom_llm_provider != Some(LITELLM_PROXY_PROVIDER) {
        return Ok(SkillSearchOutcome::UnsupportedProvider {
            reason: "query is only supported for custom_llm_provider=litellm_proxy".to_owned(),
        });
    }
    let skills = store.list_skills_for_search(principal).await?;
    search_skills(search.query, &skills, search.top_k, search.runtime)
        .await
        .map_err(HostedSkillSearchError::Search)
}

mod tests;
