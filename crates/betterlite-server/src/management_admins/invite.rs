use std::collections::BTreeSet;

use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::team_repository::decoder::decode_team_record;

use super::ports::{TeamReadError, TeamReads};
use super::scope::{AdminCaller, is_team_admin};

pub(super) const ORG_ADMIN_ROLE: &str = "org_admin";

// the same row test _user_has_admin_privileges runs before any team read
#[must_use]
pub(super) fn has_org_admin_membership(user: &LiteLLMUserTable) -> bool {
    user.organization_memberships
        .as_ref()
        .is_some_and(|memberships| {
            memberships
                .iter()
                .any(|membership| membership.user_role.as_deref() == Some(ORG_ADMIN_ROLE))
        })
}

// _org_admin_can_invite_user: the caller is org_admin in one of the target's orgs, whatever its role
#[must_use]
pub fn org_admin_can_invite_user(admin: &LiteLLMUserTable, target: &LiteLLMUserTable) -> bool {
    let Some(admin_memberships) = admin.organization_memberships.as_ref() else {
        return false;
    };
    let admin_org_ids: BTreeSet<&str> = admin_memberships
        .iter()
        .filter(|membership| membership.user_role.as_deref() == Some(ORG_ADMIN_ROLE))
        .map(|membership| membership.organization_id.as_str())
        .collect();
    if admin_org_ids.is_empty() {
        return false;
    }
    let Some(target_memberships) = target.organization_memberships.as_ref() else {
        return false;
    };
    target_memberships
        .iter()
        .any(|membership| admin_org_ids.contains(membership.organization_id.as_str()))
}

// _team_admin_can_invite_user: empty rosters reject before the query, and every fetched row validates
pub async fn team_admin_can_invite_user<Teams>(
    caller: &AdminCaller<'_>,
    admin: &LiteLLMUserTable,
    target: &LiteLLMUserTable,
    teams: &Teams,
) -> Result<bool, TeamReadError>
where
    Teams: TeamReads + ?Sized,
{
    if admin.teams.is_empty() || target.teams.is_empty() {
        return Ok(false);
    }
    let records = teams.teams_by_ids(&admin.teams).await?;
    let mut admin_team_ids: Vec<String> = Vec::new();
    for record in &records {
        let Some(team) = decode_team_record(Some(record)).map_err(TeamReadError::Record)? else {
            continue;
        };
        if is_team_admin(caller, &team) {
            admin_team_ids.push(team.fields.base.team_id.clone());
        }
    }
    if admin_team_ids.is_empty() {
        return Ok(false);
    }
    Ok(target
        .teams
        .iter()
        .any(|team_id| admin_team_ids.contains(team_id)))
}
