use betterlite_auth::auth_user_lookup::lookup::{UserLookup, UserLookupRequest};
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_auth::user_roles::UserRole;
use betterlite_db::budget_permission_rows::JsonData;
use serde::Deserialize;

use super::error::{Rejection, TeamLimitError};
use super::request::TeamLimitRequest;

// the five UserAPIKeyAuth fields the caller's ceilings read, taken from the key's serde form
#[derive(Debug, Deserialize)]
struct CallerLimits {
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    user_role: Option<UserRole>,
    #[serde(default)]
    models: Vec<JsonData>,
    #[serde(default)]
    tpm_limit: Option<i64>,
    #[serde(default)]
    rpm_limit: Option<i64>,
}

// a standalone team may not be richer than its creator; the user row is read only for a budget, and never created
pub async fn check_user_team_limits(
    request: &TeamLimitRequest<'_>,
    caller: &UserAPIKeyAuth,
    users: &UserLookup,
) -> Result<(), TeamLimitError> {
    let limits: CallerLimits = serde_json::to_value(caller)
        .and_then(serde_json::from_value)
        .map_err(TeamLimitError::Caller)?;
    let role = limits
        .user_role
        .map_or_else(|| "None".to_owned(), |found| found.to_string());
    if let (Some(requested), Some(user_id)) = (request.max_budget(), limits.user_id.as_deref()) {
        let lookup = UserLookupRequest {
            user_id: Some(user_id),
            sso_user_id: None,
            user_email: None,
            user_id_upsert: false,
            check_db_only: None,
        };
        let user = users.get_user_object(lookup).await?;
        if let Some(ceiling) = user.and_then(|found| found.max_budget)
            && requested > ceiling
        {
            return Err(TeamLimitError::from(Rejection::UserBudget {
                ceiling,
                role,
            }));
        }
    }
    // all-proxy-models is only the organization's wildcard; here it is one more literal model name
    if let Some(models) = request.models()
        && !limits.models.is_empty()
        && models.iter().any(|model| !limits.models.contains(model))
    {
        let user = limits.user_id.unwrap_or_else(|| "None".to_owned());
        let allowed = limits.models;
        return Err(TeamLimitError::from(Rejection::UserModel { allowed, user }));
    }
    check_user_rates(request, &limits, role).map_err(TeamLimitError::from)
}

fn check_user_rates(
    request: &TeamLimitRequest<'_>,
    limits: &CallerLimits,
    role: String,
) -> Result<(), Rejection> {
    if let (Some(requested), Some(ceiling)) = (request.tpm_limit(), limits.tpm_limit)
        && requested > ceiling
    {
        return Err(Rejection::UserTpm { ceiling, role });
    }
    if let (Some(requested), Some(ceiling)) = (request.rpm_limit(), limits.rpm_limit)
        && requested > ceiling
    {
        return Err(Rejection::UserRpm { ceiling, role });
    }
    Ok(())
}
