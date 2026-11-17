use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderValue, header};
use axum::response::Response;

use super::AgentSkillsState;
use super::archive_for;
use super::error::AgentSkillsError;

pub(super) async fn agent_skills_archive(
    State(state): State<AgentSkillsState>,
    Path(skill_id): Path<String>,
) -> Result<Response, AgentSkillsError> {
    if !state.index_enabled {
        return Err(AgentSkillsError::NotFound);
    }
    let Some(mut skill) = state
        .skills
        .get(&skill_id)
        .await
        .map_err(AgentSkillsError::Read)?
    else {
        return Err(AgentSkillsError::NoArchive { skill_id });
    };
    let Some(archive) = archive_for(&state, &mut skill).await? else {
        return Err(AgentSkillsError::NoArchive { skill_id });
    };
    let disposition = format!("attachment; filename=\"{skill_id}.zip\"");
    let content_disposition =
        HeaderValue::from_str(&disposition).map_err(AgentSkillsError::Header)?;
    let mut response = Response::new(Body::from(archive.content));
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/zip"),
    );
    headers.insert(header::CONTENT_DISPOSITION, content_disposition);
    Ok(response)
}
