use betterlite_auth::auth_team_lookup::lookup::{TeamLookup, TeamRequest, TeamSource};
use betterlite_auth::auth_team_lookup::team::get_team_object;
use betterlite_auth::management_routes::key_routes::KeyManagementRoute;
use betterlite_auth::user_roles::UserRole;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;

use super::caller::TeamCaller;
use super::error::TeamAccessError;
use super::permissions::member_route_allowed;
use super::roster::caller_team_role;

// can_team_member_execute_key_management_endpoint: proxy admins and personal keys skip the team read
pub async fn can_execute_key_route(
    caller: &TeamCaller,
    route: KeyManagementRoute,
    key_team_id: Option<&str>,
    teams: &TeamLookup,
) -> Result<(), TeamAccessError> {
    if caller.user_role == Some(UserRole::ProxyAdmin) {
        return Ok(());
    }
    let Some(team_id) = key_team_id else {
        return Ok(());
    };
    let team = fresh_team(teams, team_id).await?;
    let base = &team.team.fields.base;
    let role = caller_team_role(base, caller);
    if member_route_allowed(role.as_ref(), base, route.value())? {
        return Ok(());
    }
    Err(TeamAccessError::NotTeamMember {
        team_id: base.team_id.clone(),
        route: route.value().to_owned(),
    })
}

// user_belongs_to_keys_team: a personal key belongs to no team
pub async fn user_belongs_to_keys_team(
    caller: &TeamCaller,
    key_team_id: Option<&str>,
    teams: &TeamLookup,
) -> Result<bool, TeamAccessError> {
    let Some(team_id) = key_team_id else {
        return Ok(false);
    };
    let team = fresh_team(teams, team_id).await?;
    Ok(caller_team_role(&team.team.fields.base, caller).is_some())
}

// both callers pass check_db_only=True, so the cached team is never trusted for a key's membership
async fn fresh_team(
    teams: &TeamLookup,
    team_id: &str,
) -> Result<LiteLLMTeamTableCachedObj, TeamAccessError> {
    let request = TeamRequest {
        team_id,
        source: TeamSource::Writer,
        team_id_upsert: false,
    };
    get_team_object(teams, request)
        .await
        .map_err(|error| TeamAccessError::TeamLookup(Box::new(error)))
}
