use std::collections::BTreeMap;
use std::future::Future;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_error::error::ErrorClass;

use crate::auth_user_lookup::lookup::{UserLookup, UserLookupRequest};
use crate::resolved_key::auth::UserAPIKeyAuth;

use super::limits::{TeamModelBudgetLimiter, UserModelBudgetLimiter};

pub type ModelMaxBudget = BTreeMap<String, JsonData>;

// _read_user_model_max_budget: a missing id, database, row or read is None, and every failure only logs at debug
pub async fn read_user_model_max_budget(
    source: Option<&UserLookup>,
    requested_user: Option<&str>,
) -> Option<ModelMaxBudget> {
    let (Some(lookup), Some(user_id)) = (
        source.filter(|candidate| candidate.store.is_some()),
        requested_user,
    ) else {
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

async fn check_each_model<'models, Check, Pending>(
    models: &'models [String],
    mut check: Check,
) -> Result<(), ErrorClass>
where
    Check: FnMut(&'models str) -> Pending + Send,
    Pending: Future<Output = Result<bool, ErrorClass>> + Send,
{
    for model in models {
        check(model.as_str()).await?;
    }
    Ok(())
}

// _check_user_model_budget: every model in order, including duplicates, and the limiter's bool is not a verdict
pub async fn check_user_model_budget(
    limiter: &dyn UserModelBudgetLimiter,
    token: &UserAPIKeyAuth,
    models: &[String],
) -> Result<(), ErrorClass> {
    if let (Some(user_id), Some(budget)) = (
        token.view.token.user_id.as_deref(),
        token.extra.user_model_max_budget.as_ref(),
    ) && !budget.0.is_empty()
    {
        return check_each_model(models, |model| {
            limiter.is_user_within_model_budget(user_id, &budget.0, model)
        })
        .await;
    }
    Ok(())
}

// _check_team_model_budget: the key's own map travels unchanged so the limiter can apply its overrides
pub async fn check_team_model_budget(
    limiter: &dyn TeamModelBudgetLimiter,
    token: &UserAPIKeyAuth,
    models: &[String],
) -> Result<(), ErrorClass> {
    let key_budget = &token.view.token.model_max_budget.0;
    let (Some(team_id), Some(budget)) = (
        token.view.token.team_id.as_deref(),
        token.view.team_model_max_budget.as_ref(),
    ) else {
        return Ok(());
    };
    if budget.0.is_empty() {
        return Ok(());
    }
    check_each_model(models, |model| {
        limiter.is_team_within_model_budget(team_id, &budget.0, Some(key_budget), model)
    })
    .await
}
