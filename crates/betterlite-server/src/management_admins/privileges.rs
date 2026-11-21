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
    if caller.proxy_admin {
        return true;
    }
    let Some(teams) = teams else {
        return false;
    };
    let Some(user_id) = caller.user_id else {
        return false;
    };
    match privileges_block(user_id, caller, users, teams, cache).await {
        Ok(granted) => granted,
        Err(error) => {
            tracing::debug!("Error checking admin privileges for user {user_id}: {error}");
            false
        }
    }
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

// admin_can_invite_user: the same short circuits, one cache for both reads, caller before target
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
    if caller.proxy_admin {
        return true;
    }
    let Some(teams) = teams else {
        return false;
    };
    let Some(user_id) = caller.user_id else {
        return false;
    };
    match invite_block(target_user_id, caller, users, teams, cache).await {
        Ok(granted) => granted,
        Err(error) => {
            tracing::debug!("Error checking invite permission for user {user_id}: {error}");
            false
        }
    }
}

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
