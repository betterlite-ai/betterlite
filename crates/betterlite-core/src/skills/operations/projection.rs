use betterlite_anthropic::skills::wire::Skill;
use betterlite_db::managed_resource_rows::skills::SkillTable;
use betterlite_db::managed_resource_rows::wire::RowTimestamp;

use super::HostedSkillOpsError;

const SKILL_RECORD_TYPE: &str = "skill";
const DEFAULT_SOURCE: &str = "custom";

// LiteLLM's db_skill_to_response: a missing date becomes an empty string, never a null
pub(super) fn db_skill_to_response(row: &SkillTable) -> Result<Skill, HostedSkillOpsError> {
    Ok(Skill {
        id: row.skill_id.clone(),
        created_at: iso_datetime(row.created_at.as_ref())?,
        display_title: row.display_title.clone(),
        latest_version: row.latest_version.clone(),
        source: source_name(&row.source).to_owned(),
        r#type: SKILL_RECORD_TYPE.to_owned(),
        updated_at: iso_datetime(row.updated_at.as_ref())?,
        description: row.description.clone(),
        search_score: None,
    })
}

// LiteLLM's `db_skill.source or "custom"` also falls back on an empty source
const fn source_name(source: &str) -> &str {
    if source.is_empty() {
        return DEFAULT_SOURCE;
    }
    source
}

fn iso_datetime(timestamp: Option<&RowTimestamp>) -> Result<String, HostedSkillOpsError> {
    let Some(stamp) = timestamp else {
        return Ok(String::new());
    };
    let encoded = serde_json::to_value(stamp).map_err(HostedSkillOpsError::Timestamp)?;
    serde_json::from_value(encoded).map_err(HostedSkillOpsError::Timestamp)
}
