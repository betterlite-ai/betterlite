use betterlite_db::team_rows::member::{Member, MemberRole};
use betterlite_db::team_rows::team::TeamBase;

use super::caller::TeamCaller;

// key_management_endpoints.py _get_user_in_team: the first entry carrying the caller's user id
#[must_use]
pub fn roster_member<'team>(
    members: &'team [Member],
    user_id: Option<&str>,
) -> Option<&'team Member> {
    members
        .iter()
        .find(|member| member.base.user_id.is_some() && member.base.user_id.as_deref() == user_id)
}

// _get_caller_team_role: the team's own service account acts as a member whatever the roster says
#[must_use]
pub fn caller_team_role(team: &TeamBase<String>, caller: &TeamCaller) -> Option<MemberRole> {
    if caller.team_service_account && caller.team_id.as_deref() == Some(team.team_id.as_str()) {
        return Some(MemberRole::User);
    }
    roster_member(&team.members_with_roles, caller.user_id.as_deref())
        .map(|member| member.role.clone())
}

// common_utils.py _team_member_has_permission: the team's explicit list only, no baseline routes
#[must_use]
pub fn member_has_permission(
    caller: &TeamCaller,
    team: &TeamBase<String>,
    permission: &str,
) -> bool {
    let granted = team
        .team_member_permissions
        .iter()
        .flatten()
        .any(|listed| listed == permission);
    granted && roster_member(&team.members_with_roles, caller.user_id.as_deref()).is_some()
}
