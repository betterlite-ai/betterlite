use betterlite_auth::management_routes::key_routes::KeyManagementRoute;
use betterlite_auth::user_roles::UserRole;
use betterlite_config::sections::key_access::TeamKeyGeneration;
use betterlite_db::team_rows::member::MemberRole;
use betterlite_db::team_rows::team::TeamBase;

use super::error::KeyAccessError;
use crate::team_access::caller::TeamCaller;
use crate::team_access::permissions::member_route_allowed;
use crate::team_access::roster::{caller_team_role, roster_member};

// _team_key_operation_team_member_check: the assigned user is checked before the proxy-admin shortcut
pub fn team_member_check(
    assigned_user_id: Option<&str>,
    team: &TeamBase<String>,
    caller: &TeamCaller,
    config: &TeamKeyGeneration,
    route: KeyManagementRoute,
) -> Result<(), KeyAccessError> {
    if let Some(assigned) = assigned_user_id
        && roster_member(&team.members_with_roles, Some(assigned)).is_none()
    {
        return Err(KeyAccessError::AssignedNotMember {
            user_id: assigned.to_owned(),
            team_id: team.team_id.clone(),
        });
    }
    let role = caller_team_role(team, caller);
    if caller.user_role == Some(UserRole::ProxyAdmin) {
        return Ok(());
    }
    let Some(member_role) = role else {
        return Err(KeyAccessError::CallerNotMember {
            team_id: team.team_id.clone(),
        });
    };
    let role_name = match member_role {
        MemberRole::Admin => "admin",
        MemberRole::User => "user",
    };
    if let Some(allowed) = &config.allowed_team_member_roles
        && !allowed.iter().any(|listed| listed == role_name)
    {
        return Err(KeyAccessError::RoleNotAllowed {
            role: role_name.to_owned(),
            allowed: allowed.clone(),
        });
    }
    member_route_allowed(Some(&member_role), team, route.value())
        .map(drop)
        .map_err(KeyAccessError::from)
}
