use betterlite_auth::management_routes::key_routes::{KEY_ENUM, KeyManagementRoute};
use betterlite_auth::route_match::templates::check_route_access;
use betterlite_auth::user_roles::UserRole;
use betterlite_db::team_rows::member::MemberRole;
use betterlite_db::team_rows::team::TeamBase;

use super::caller::TeamCaller;
use super::error::TeamAccessError;
use super::roster::caller_team_role;

// BASELINE_TEAM_MEMBER_PERMISSIONS, also what default_team_member_permissions lists
pub const BASELINE_PERMISSIONS: [KeyManagementRoute; 2] =
    [KeyManagementRoute::KeyInfo, KeyManagementRoute::KeyHealth];

// an explicit list, even [], keeps the baseline; an absent one is the baseline alone
pub fn member_permissions(
    team: &TeamBase<String>,
) -> Result<Vec<KeyManagementRoute>, TeamAccessError> {
    let Some(explicit) = team.team_member_permissions.as_deref() else {
        return Ok(BASELINE_PERMISSIONS.to_vec());
    };
    let parsed = explicit
        .iter()
        .map(|value| {
            KEY_ENUM
                .iter()
                .copied()
                .find(|route| route.value() == value.as_str())
                .ok_or_else(|| TeamAccessError::UnknownPermission {
                    value: value.clone(),
                })
        })
        .collect::<Result<Vec<KeyManagementRoute>, TeamAccessError>>()?;
    let mut permissions = Vec::new();
    for route in parsed.into_iter().chain(BASELINE_PERMISSIONS) {
        if !permissions.contains(&route) {
            permissions.push(route);
        }
    }
    Ok(permissions)
}

// does_team_member_have_permissions_for_endpoint: a denied member raises instead of answering false
pub fn member_route_allowed(
    role: Option<&MemberRole>,
    team: &TeamBase<String>,
    route: &str,
) -> Result<bool, TeamAccessError> {
    match role {
        None => Ok(false),
        Some(MemberRole::Admin) => Ok(true),
        Some(MemberRole::User) => {
            let allowed: Vec<&str> = member_permissions(team)?
                .into_iter()
                .map(KeyManagementRoute::value)
                .collect();
            if check_route_access(route, &allowed) {
                return Ok(true);
            }
            Err(TeamAccessError::MemberRoute {
                team_id: team.team_id.clone(),
                route: route.to_owned(),
                allowed: allowed.into_iter().map(str::to_owned).collect(),
            })
        }
    }
}

// enforce_member_can_assign_access_groups: an exact opt-in entry, never a route match
pub fn enforce_group_assignment(
    caller: &TeamCaller,
    team: Option<&TeamBase<String>>,
    access_group_ids: Option<&[String]>,
) -> Result<(), TeamAccessError> {
    let Some(requested) = access_group_ids.filter(|ids| !ids.is_empty()) else {
        return Ok(());
    };
    if caller.user_role == Some(UserRole::ProxyAdmin) {
        return Ok(());
    }
    let Some(found) = team else {
        let mut denied = requested.to_vec();
        denied.sort();
        return Err(TeamAccessError::PersonalKeyGroups { denied });
    };
    let opted_in = match caller_team_role(found, caller) {
        None => false,
        Some(MemberRole::Admin) => return Ok(()),
        Some(MemberRole::User) => {
            member_permissions(found)?.contains(&KeyManagementRoute::KeyAccessGroupAssignment)
        }
    };
    if opted_in {
        return Ok(());
    }
    Err(TeamAccessError::GroupOptIn {
        team_id: found.team_id.clone(),
    })
}
