use betterlite_auth::auth_team_lookup::lookup::TeamLookup;
use betterlite_auth::resolved_key::auth::{UserAPIKeyAuth, user_api_key_has_admin_view};
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::key_hash::KeyHash;

use super::error::{KeyListFiltersError, KeyListReadError};
use super::ports::KeyListFiltersStore;
use super::selectors::KeyListSelectors;
use crate::key_info::access::can_user_query_key_info;
use crate::team_access::caller::TeamCaller;

// validate_key_list_check: an admin view skips the check, everyone else lists only what their row owns
pub async fn validate_key_list_check(
    caller: &UserAPIKeyAuth,
    selectors: &KeyListSelectors<'_>,
    store: &dyn KeyListFiltersStore,
    teams: &TeamLookup,
) -> Result<Option<LiteLLMUserTable>, KeyListFiltersError> {
    if user_api_key_has_admin_view(caller) {
        return Ok(None);
    }
    let team_caller = TeamCaller::try_from(caller).map_err(KeyListReadError::Caller)?;
    let Some(caller_id) = team_caller.user_id.as_deref() else {
        return Err(KeyListFiltersError::NoCallerRow);
    };
    let Some(user_row) = store.user_row(caller_id).await? else {
        return Err(KeyListFiltersError::NoCallerRow);
    };
    permit(selectors, &user_row)?;
    permit_key_hash(caller, selectors, store, teams, &team_caller).await?;
    Ok(Some(user_row))
}

// LiteLLM's `if value:` skips an absent or empty selector; the rest must fall inside the row's scope, in check order
fn permit(
    selectors: &KeyListSelectors<'_>,
    user_row: &LiteLLMUserTable,
) -> Result<(), KeyListFiltersError> {
    if selectors
        .user_id
        .filter(|text| !text.is_empty())
        .is_some_and(|wanted| user_row.user_id != wanted)
    {
        return Err(KeyListFiltersError::ForeignUser);
    }
    if let Some(wanted) = selectors.team_id.filter(|text| !text.is_empty()) {
        let member = user_row.teams.iter().any(|team| team == wanted);
        if !member {
            return Err(KeyListFiltersError::ForeignTeam);
        }
    }
    // any membership role reaches the organization's keys, and an absent list reaches none
    if let Some(wanted) = selectors.organization_id.filter(|text| !text.is_empty()) {
        let memberships = user_row
            .organization_memberships
            .as_deref()
            .unwrap_or_default();
        let member = memberships
            .iter()
            .any(|membership| membership.organization_id == wanted);
        if !member {
            return Err(KeyListFiltersError::ForeignOrganization);
        }
    }
    Ok(())
}

async fn permit_key_hash(
    caller: &UserAPIKeyAuth,
    selectors: &KeyListSelectors<'_>,
    store: &dyn KeyListFiltersStore,
    teams: &TeamLookup,
    team_caller: &TeamCaller,
) -> Result<(), KeyListFiltersError> {
    let Some(wanted) = selectors.key_hash.filter(|text| !text.is_empty()) else {
        return Ok(());
    };
    // LiteLLM turns every failed key read into the same not-found denial
    let Ok(found) = store.key_by_hash(&KeyHash::from(wanted)).await else {
        return Err(KeyListFiltersError::KeyHashNotFound);
    };
    let Some(key_row) = found else {
        return Err(KeyListFiltersError::KeyHashNotFound);
    };
    let allowed = can_user_query_key_info(caller, Some(wanted), &key_row, teams)
        .await
        .map_err(KeyListReadError::Access)?;
    if allowed {
        Ok(())
    } else {
        Err(KeyListFiltersError::KeyInfoForbidden {
            role: team_caller.user_role,
        })
    }
}
