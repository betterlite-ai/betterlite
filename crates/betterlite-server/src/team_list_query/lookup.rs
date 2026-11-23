use betterlite_auth::auth_user_lookup::error::UserLookupError;
use betterlite_auth::auth_user_lookup::lookup::{UserLookup, UserLookupRequest};
use betterlite_db::identity_rows::user::LiteLLMUserTable;

use super::{TeamListQueryError, TeamListRejection};

// the membership role value LitellmUserRoles.ORG_ADMIN carries
const ORG_ADMIN_ROLE: &str = "org_admin";

// get_user_object without an upsert: the caller is read, never created
pub(super) const fn request(user_id: &str, check_db_only: Option<bool>) -> UserLookupRequest<'_> {
    UserLookupRequest {
        user_id: Some(user_id),
        sso_user_id: None,
        user_email: None,
        user_id_upsert: false,
        check_db_only,
    }
}

pub(super) fn org_ids_where_admin(user: &LiteLLMUserTable) -> Vec<String> {
    user.organization_memberships
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .filter(|membership| membership.user_role.as_deref() == Some(ORG_ADMIN_ROLE))
        .map(|membership| membership.organization_id.clone())
        .collect()
}

// _get_org_admin_org_ids answers None to both ValueError forms; an outage keeps propagating
pub async fn org_admin_org_ids(
    lookup: &UserLookup,
    user_id: &str,
) -> Result<Option<Vec<String>>, TeamListQueryError> {
    let found = match lookup.get_user_object(request(user_id, None)).await {
        Ok(found) => found,
        Err(UserLookupError::NotFound(_) | UserLookupError::UserMissing(_)) => return Ok(None),
        Err(other) => return Err(TeamListQueryError::from(other)),
    };
    let Some(user) = found else {
        return Ok(None);
    };
    let org_ids = org_ids_where_admin(&user);
    Ok((!org_ids.is_empty()).then_some(org_ids))
}

// check_db_only reads the user's own row from the writer and never the cache
pub async fn user_team_ids_from_db(
    lookup: &UserLookup,
    user_id: &str,
) -> Result<Vec<String>, TeamListQueryError> {
    let found = match lookup.get_user_object(request(user_id, Some(true))).await {
        Ok(found) => found,
        Err(UserLookupError::NotFound(_)) => return Ok(Vec::new()),
        Err(other) => return Err(TeamListQueryError::from(other)),
    };
    Ok(found.map(|user| user.teams).unwrap_or_default())
}

// a requested user's team membership filter; a missing or unreadable row is 404
pub async fn requested_user_teams(
    lookup: &UserLookup,
    user_id: &str,
) -> Result<Vec<String>, TeamListQueryError> {
    let not_found = TeamListQueryError::from(TeamListRejection::UserNotFound {
        user_id: user_id.to_owned(),
    });
    match lookup.get_user_object(request(user_id, None)).await {
        Ok(Some(user)) => Ok(user.teams),
        Ok(None) => Err(not_found),
        Err(UserLookupError::NotFound(_) | UserLookupError::UserMissing(_)) => Err(not_found),
        Err(other) => Err(TeamListQueryError::from(other)),
    }
}
