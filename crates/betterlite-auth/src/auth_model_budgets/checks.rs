use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_error::error::ErrorClass;

use crate::auth_user_lookup::lookup::{UserLookup, UserLookupRequest};
use crate::resolved_key::auth::UserAPIKeyAuth;

use super::limits::{TeamModelBudgetLimiter, UserModelBudgetLimiter};

pub type ModelMaxBudget = BTreeMap<String, JsonData>;

// _read_user_model_max_budget: a missing id, database, row or read is None, and every failure only logs at debug
pub async fn read_user_model_max_budget(
    lookup: Option<&UserLookup>,
    user_id: Option<&str>,
) -> Option<ModelMaxBudget> {
    let (Some(lookup), Some(user_id)) = (lookup, user_id) else {
        return None;
    };
    let request = UserLookupRequest {
        user_id: Some(user_id),
        sso_user_id: None,
        user_email: None,
        user_id_upsert: false,
        check_db_only: None,
    };
    match lookup.get_user_object(request).await {
        Ok(Some(user)) => user.model_max_budget.0.map(|budget| budget.0),
        Ok(None) => None,
        Err(error) => {
            // the row's budget is optional, so an unreadable user never refuses the request
            tracing::debug!(error = %error, "Unable to read user for the per-model budget check");
            None
        }
    }
}

// _check_user_model_budget: every model in order, including duplicates, and the limiter's bool is not a verdict
pub async fn check_user_model_budget(
    limiter: &dyn UserModelBudgetLimiter,
    token: &UserAPIKeyAuth,
    models: &[String],
) -> Result<(), ErrorClass> {
    let Some(user_id) = token.view.token.user_id.as_deref() else {
        return Ok(());
    };
    let Some(budget) = token.extra.user_model_max_budget.as_ref() else {
        return Ok(());
    };
    if budget.0.is_empty() {
        return Ok(());
    }
    for model in models {
        limiter
            .is_user_within_model_budget(user_id, &budget.0, model)
            .await?;
    }
    Ok(())
}

// _check_team_model_budget: the key's own map travels unchanged so the limiter can apply its overrides
pub async fn check_team_model_budget(
    limiter: &dyn TeamModelBudgetLimiter,
    token: &UserAPIKeyAuth,
    models: &[String],
) -> Result<(), ErrorClass> {
    let Some(team_id) = token.view.token.team_id.as_deref() else {
        return Ok(());
    };
    let Some(budget) = token.view.team_model_max_budget.as_ref() else {
        return Ok(());
    };
    if budget.0.is_empty() {
        return Ok(());
    }
    let key_budget = &token.view.token.model_max_budget.0;
    for model in models {
        limiter
            .is_team_within_model_budget(team_id, &budget.0, Some(key_budget), model)
            .await?;
    }
    Ok(())
}
