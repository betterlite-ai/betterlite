use betterlite_auth::management_routes::key_routes::KeyManagementRoute;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::team_repository::decoder::{TeamRecordError, decode_team_record};
use betterlite_db::team_rows::team::LiteLLMTeamTable;

use super::error::{KeyListFiltersError, KeyListReadError};
use crate::management_admins::ports::TeamReads;
use crate::team_access::access::is_team_admin;
use crate::team_access::caller::TeamCaller;
use crate::team_access::roster::member_has_permission;

// _fetch_user_team_objects: one team query for the row's teams list, every row decoded or an error
pub async fn fetch_user_team_objects(
    user: Option<&LiteLLMUserTable>,
    teams: &dyn TeamReads,
) -> Result<Vec<LiteLLMTeamTable>, KeyListFiltersError> {
    let Some(user) = user else {
        return Ok(Vec::new());
    };
    if user.teams.is_empty() {
        return Ok(Vec::new());
    }
    let records = teams
        .teams_by_ids(&user.teams)
        .await
        .map_err(KeyListReadError::Teams)?;
    let decoded: Result<Vec<Option<LiteLLMTeamTable>>, TeamRecordError> = records
        .iter()
        .map(|record| decode_team_record(Some(record)))
        .collect();
    Ok(decoded
        .map_err(KeyListReadError::Team)?
        .into_iter()
        .flatten()
        .collect())
}

// _get_admin_team_ids_from_objects: every returned team whose roster names the caller an admin
#[must_use]
pub fn admin_team_ids(caller: &TeamCaller, teams: &[LiteLLMTeamTable]) -> Vec<String> {
    teams
        .iter()
        .filter(|team| is_team_admin(caller, &team.fields.base))
        .map(|team| team.fields.base.team_id.clone())
        .collect()
}

// _get_team_ids_with_key_list_permission_from_objects: non-admin teams that grant /key/list
#[must_use]
pub fn list_permission_team_ids(caller: &TeamCaller, teams: &[LiteLLMTeamTable]) -> Vec<String> {
    team_ids_where(teams, &|team| {
        let base = &team.fields.base;
        !is_team_admin(caller, base)
            && member_has_permission(caller, base, KeyManagementRoute::KeyList.value())
    })
}

// _get_member_team_ids_from_objects: any roster entry carrying the caller's user id, whatever its role
#[must_use]
pub fn member_team_ids(caller: &TeamCaller, teams: &[LiteLLMTeamTable]) -> Vec<String> {
    team_ids_where(teams, &|team| {
        team.fields
            .base
            .members_with_roles
            .iter()
            .any(|entry| entry.base.user_id.is_some() && entry.base.user_id == caller.user_id)
    })
}

fn team_ids_where(
    teams: &[LiteLLMTeamTable],
    keep: &dyn Fn(&LiteLLMTeamTable) -> bool,
) -> Vec<String> {
    teams
        .iter()
        .filter(|team| keep(team))
        .map(|team| team.fields.base.team_id.clone())
        .collect()
}
