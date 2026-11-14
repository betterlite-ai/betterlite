use super::GROUP_SCHEMA;
use super::error::ProjectionError;
use super::member::project_member;
use super::timestamps::resource_meta;
use crate::scim_records::group::Group;
use betterlite_db::team_rows::team::LiteLLMTeamTable;

// the roster is walked as stored, so a member whose user row is gone still appears
pub fn project_group(team: &LiteLLMTeamTable) -> Result<Group, ProjectionError> {
    let fields = &team.fields;
    let members = fields
        .base
        .members_with_roles
        .iter()
        .map(|member| {
            project_member(
                member.base.user_id.as_deref(),
                member.base.user_email.as_deref(),
            )
        })
        .collect();
    Ok(Group {
        schemas: vec![GROUP_SCHEMA.to_owned()],
        id: Some(fields.base.team_id.clone()),
        external_id: None,
        meta: Some(resource_meta(
            "Group",
            fields.created_at,
            fields.updated_at,
        )?),
        // displayName is required, so a team without an alias fails instead of borrowing its id
        display_name: fields
            .base
            .team_alias
            .clone()
            .ok_or(ProjectionError::GroupDisplayName)?,
        members: Some(members),
    })
}
