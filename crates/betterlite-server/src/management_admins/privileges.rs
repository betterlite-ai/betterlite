use std::sync::Arc;

use betterlite_auth::auth_user_lookup::cache::UserCache;
use betterlite_auth::auth_user_lookup::lookup::UserLookupRequest;
use betterlite_db::team_repository::decoder::decode_team_record;

use super::invite::{
    has_org_admin_membership, org_admin_can_invite_user, team_admin_can_invite_user,
};
use super::ports::{AdminUserReads, TeamReadError, TeamReads, UserReadError};
use super::scope::{AdminCaller, is_team_admin};

#[derive(Debug, thiserror::Error)]
enum ScopeError {
    #[error(transparent)]
    User(#[from] UserReadError),
    #[error(transparent)]
    Team(#[from] TeamReadError),
}

// which scope block a check runs after the shared short circuits
enum ScopeCheck<'check> {
    Privileges,
    Invite(&'check str),
}

// the two public checks share every short circuit and the failure log, and differ only in their block
async fn check_scope<Users, Teams>(
    caller: &AdminCaller<'_>,
    users: &Users,
    teams: Option<&Teams>,
    cache: Option<Arc<dyn UserCache>>,
    which: ScopeCheck<'_>,
) -> bool
where
    Users: AdminUserReads + ?Sized,
    Teams: TeamReads + ?Sized,
{
    if caller.proxy_admin {
        return true;
    }
    let Some(teams) = teams else {
        return false;
    };
    let Some(user_id) = caller.user_id else {
        return false;
    };
    let (what, outcome) = match which {
        ScopeCheck::Privileges => (
            "admin privileges",
            privileges_block(user_id, caller, users, teams, cache).await,
        ),
        ScopeCheck::Invite(target_user_id) => (
            "invite permission",
            invite_block(target_user_id, caller, users, teams, cache).await,
        ),
    };
    match outcome {
        Ok(granted) => granted,
        Err(error) => {
            tracing::debug!("Error checking {what} for user {user_id}: {error}");
            false
        }
    }
}

// _user_has_admin_privileges: proxy admin first, then any org_admin membership, then any roster team
pub async fn user_has_admin_privileges<Users, Teams>(
    caller: &AdminCaller<'_>,
    users: &Users,
    teams: Option<&Teams>,
    cache: Option<Arc<dyn UserCache>>,
) -> bool
where
    Users: AdminUserReads + ?Sized,
    Teams: TeamReads + ?Sized,
{
    check_scope(caller, users, teams, cache, ScopeCheck::Privileges).await
}

async fn privileges_block<Users, Teams>(
    user_id: &str,
    caller: &AdminCaller<'_>,
    users: &Users,
    teams: &Teams,
    cache: Option<Arc<dyn UserCache>>,
) -> Result<bool, ScopeError>
where
    Users: AdminUserReads + ?Sized,
    Teams: TeamReads + ?Sized,
{
    let cache = match cache {
        Some(cache) => cache,
        None => users.fresh_cache()?,
    };
    let request = UserLookupRequest {
        user_id: Some(user_id),
        sso_user_id: None,
        user_email: None,
        user_id_upsert: false,
        check_db_only: None,
    };
    let Some(user) = users.user_object(request, cache).await? else {
        return Ok(false);
    };
    if has_org_admin_membership(&user) {
        return Ok(true);
    }
    if user.teams.is_empty() {
        return Ok(false);
    }
    let records = teams.teams_by_ids(&user.teams).await?;
    for record in &records {
        let Some(team) = decode_team_record(Some(record)).map_err(TeamReadError::Record)? else {
            continue;
        };
        if is_team_admin(caller, &team) {
            return Ok(true);
        }
    }
    Ok(false)
}

// admin_can_invite_user: the shared short circuits, then the org check before the team intersection
pub async fn admin_can_invite_user<Users, Teams>(
    target_user_id: &str,
    caller: &AdminCaller<'_>,
    users: &Users,
    teams: Option<&Teams>,
    cache: Option<Arc<dyn UserCache>>,
) -> bool
where
    Users: AdminUserReads + ?Sized,
    Teams: TeamReads + ?Sized,
{
    check_scope(
        caller,
        users,
        teams,
        cache,
        ScopeCheck::Invite(target_user_id),
    )
    .await
}

// one cache serves both reads, and the caller row is loaded before the target row
async fn invite_block<Users, Teams>(
    target_user_id: &str,
    caller: &AdminCaller<'_>,
    users: &Users,
    teams: &Teams,
    cache: Option<Arc<dyn UserCache>>,
) -> Result<bool, ScopeError>
where
    Users: AdminUserReads + ?Sized,
    Teams: TeamReads + ?Sized,
{
    let Some(admin_user_id) = caller.user_id else {
        return Ok(false);
    };
    let cache = match cache {
        Some(cache) => cache,
        None => users.fresh_cache()?,
    };
    let admin_request = UserLookupRequest {
        user_id: Some(admin_user_id),
        sso_user_id: None,
        user_email: None,
        user_id_upsert: false,
        check_db_only: None,
    };
    let Some(admin) = users.user_object(admin_request, Arc::clone(&cache)).await? else {
        return Ok(false);
    };
    let target_request = UserLookupRequest {
        user_id: Some(target_user_id),
        sso_user_id: None,
        user_email: None,
        user_id_upsert: false,
        check_db_only: None,
    };
    let Some(target) = users.user_object(target_request, cache).await? else {
        return Ok(false);
    };
    if org_admin_can_invite_user(&admin, &target) {
        return Ok(true);
    }
    let granted = team_admin_can_invite_user(caller, &admin, &target, teams).await?;
    Ok(granted)
}
