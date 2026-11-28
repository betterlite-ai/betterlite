use axum::Json;
use axum::extract::{Request, State};
use betterlite_core::api::stored_skill_reads::stored_skill::StoredSkill;
use betterlite_http::skill_archive::SkillArchive;
use serde::Serialize;

use super::AgentSkillsState;
use super::archive_for;
use super::error::AgentSkillsError;
use super::names::{base_name, deduplicate, entry_description};
use crate::middleware::request_root_paths::{get_request_root_path, request_base_url};

const AGENT_SKILLS_DISCOVERY_SCHEMA: &str =
    "https://schemas.agentskills.io/discovery/0.2.0/schema.json";
const ARCHIVE_ENTRY_TYPE: &str = "archive";

#[derive(Debug, Serialize)]
pub(super) struct AgentSkillsIndex {
    #[serde(rename = "$schema")]
    discovery_schema: &'static str,
    skills: Vec<AgentSkillsIndexEntry>,
}

#[derive(Debug, Serialize)]
struct AgentSkillsIndexEntry {
    name: String,
    #[serde(rename = "type")]
    entry_type: &'static str,
    description: String,
    url: String,
    digest: String,
}

pub(super) async fn agent_skills_index(
    State(state): State<AgentSkillsState>,
    request: Request,
) -> Result<Json<AgentSkillsIndex>, AgentSkillsError> {
    if !state.index_enabled {
        return Err(AgentSkillsError::NotFound);
    }
    let mut skills = state.skills.list().await.map_err(AgentSkillsError::List)?;
    skills.reverse();
    let installable = installable_skills(&state, skills).await?;
    let base_names: Vec<String> = installable
        .iter()
        .map(|(skill, archive)| base_name(skill, archive))
        .collect();
    let names = deduplicate(&base_names);
    let root_path = get_request_root_path(request.extensions(), "");
    let base_url = request_base_url(&request, &root_path);
    let entries = installable
        .into_iter()
        .zip(names)
        .map(|((skill, archive), name)| {
            let description = entry_description(&skill, &archive, &name);
            let url = archive_url(&state, &skill.skill_id, &base_url, &root_path);
            AgentSkillsIndexEntry {
                name,
                entry_type: ARCHIVE_ENTRY_TYPE,
                description,
                url,
                digest: archive.digest,
            }
        })
        .collect();
    Ok(Json(AgentSkillsIndex {
        discovery_schema: AGENT_SKILLS_DISCOVERY_SCHEMA,
        skills: entries,
    }))
}

async fn installable_skills(
    state: &AgentSkillsState,
    skills: Vec<StoredSkill>,
) -> Result<Vec<(StoredSkill, SkillArchive)>, AgentSkillsError> {
    let mut built = Vec::new();
    for mut skill in skills {
        if let Some(archive) = archive_for(state, &mut skill).await? {
            built.push((skill, archive));
        }
    }
    Ok(built)
}

fn archive_url(
    state: &AgentSkillsState,
    skill_id: &str,
    base_url: &str,
    root_path: &str,
) -> String {
    state.custom_urls.custom_route_url(
        base_url,
        root_path,
        &format!("v1/skills/{skill_id}/archive"),
    )
}
