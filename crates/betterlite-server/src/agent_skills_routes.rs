mod archive;
mod archive_cache;
pub mod archives;
mod error;
mod index;
mod names;
pub mod skills;
mod tests;

use std::sync::Arc;

use axum::Router;
use axum::routing::get;
use betterlite_context::dynamic_logging_cache::Clock;
use betterlite_context::local_ttl_cache::CacheError;
use betterlite_core::api::stored_skill_reads::stored_skill::StoredSkill;
use betterlite_http::skill_archive::SkillArchive;
use tokio::task::JoinSet;

use crate::custom_url::CustomUrls;

use archive::agent_skills_archive;
use archive_cache::ArchiveCache;
use archives::ArchiveBuilder;
use error::AgentSkillsError;
use index::agent_skills_index;
use skills::StoredSkills;

const AGENT_SKILLS_INDEX_PATH: &str = "/.well-known/agent-skills/index.json";
const SKILLS_INDEX_ALIAS_PATH: &str = "/.well-known/skills/index.json";
const SKILL_ARCHIVE_PATH: &str = "/v1/skills/{skill_id}/archive";
// LiteLLM's MAX_CACHED_ARCHIVE_BYTES: 512 KiB
const MAX_CACHED_ARCHIVE_BYTES: usize = 0x0008_0000;

#[derive(Clone)]
pub struct AgentSkillsState {
    index_enabled: bool,
    skills: Arc<dyn StoredSkills>,
    archives: Arc<dyn ArchiveBuilder>,
    archive_cache: Arc<ArchiveCache>,
    custom_urls: CustomUrls,
}

impl AgentSkillsState {
    pub fn new(
        index_enabled: bool,
        skills: Arc<dyn StoredSkills>,
        archives: Arc<dyn ArchiveBuilder>,
        custom_urls: CustomUrls,
        clock: Clock,
    ) -> Result<Self, CacheError> {
        Ok(Self {
            index_enabled,
            skills,
            archives,
            archive_cache: Arc::new(ArchiveCache::new(clock)?),
            custom_urls,
        })
    }
}

pub fn routes(state: AgentSkillsState) -> Router {
    Router::new()
        .route(AGENT_SKILLS_INDEX_PATH, get(agent_skills_index))
        .route(SKILLS_INDEX_ALIAS_PATH, get(agent_skills_index))
        .route(SKILL_ARCHIVE_PATH, get(agent_skills_archive))
        .with_state(state)
}

// LiteLLM's _archive_for: the stored upload, then the cache by id and updated_at, then a blocking rebuild
async fn archive_for(
    state: &AgentSkillsState,
    skill: &mut StoredSkill,
) -> Result<Option<SkillArchive>, AgentSkillsError> {
    let Some(upload) = skill.file_content.take() else {
        return Ok(None);
    };
    let key = cache_key(skill)?;
    if let Some(lookup_key) = &key
        && let Some(cached) = state.archive_cache.lookup(lookup_key)
    {
        return Ok(Some(cached));
    }
    let Some(archive) = build_archive(&skill.skill_id, &state.archives, upload).await? else {
        return Ok(None);
    };
    if let Some(store_key) = key
        && archive.content.len() <= MAX_CACHED_ARCHIVE_BYTES
    {
        state.archive_cache.store(&store_key, &archive);
    }
    Ok(Some(archive))
}

// pydantic writes a stored datetime as its ISO text, which is the cache key's second half; no updated_at means no key
fn cache_key(skill: &StoredSkill) -> Result<Option<String>, AgentSkillsError> {
    let Some(stamp) = &skill.updated_at else {
        return Ok(None);
    };
    let encoded = serde_json::to_value(stamp).map_err(AgentSkillsError::Timestamp)?;
    let updated_at: String =
        serde_json::from_value(encoded).map_err(AgentSkillsError::Timestamp)?;
    Ok(Some(format!("{}:{updated_at}", skill.skill_id)))
}

// A build that yields nothing is LiteLLM's skipped skill, and it is logged on every index call since nothing caches it
async fn build_archive(
    skill_id: &str,
    builder: &Arc<dyn ArchiveBuilder>,
    upload: Vec<u8>,
) -> Result<Option<SkillArchive>, AgentSkillsError> {
    let worker = Arc::clone(builder);
    let mut tasks = JoinSet::new();
    tasks.spawn_blocking(move || worker.build(&upload));
    let built = tasks
        .join_next()
        .await
        .ok_or(AgentSkillsError::MissingBuild)?
        .map_err(AgentSkillsError::Build)?;
    let Some(archive) = built else {
        tracing::warn!(
            "Agent Skills index: skipping skill {skill_id}, its upload is not a zip holding SKILL.md at the root of a single top-level folder"
        );
        return Ok(None);
    };
    Ok(Some(archive))
}
