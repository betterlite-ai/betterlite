use std::convert::Infallible;
use std::future::Future;

use betterlite_auth::model_budget_limiter::ModelBudgetLimiter;
use betterlite_auth::model_budget_limiter::cache::ModelBudgetCache;
use betterlite_auth::model_budget_limiter::enforcement::{
    BudgetSubject, subject_within_model_budget,
};
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_error::error::{ErrorClass, class_message};
use secrecy::ExposeSecret as _;

use super::error::SummaryLimitsError;
use super::fields::{BudgetMap, LimitFields};

// proxy_server.model_max_budget_limiter: the per-model budget check of one key, user, team or end user
pub trait SummaryModelBudgets: Sync {
    fn within_model_budget(
        &self,
        subject: &BudgetSubject<'_>,
        model_max_budget: &BudgetMap,
        model: &str,
    ) -> impl Future<Output = Result<(), ErrorClass>> + Send;
}

impl<Cache: ModelBudgetCache> SummaryModelBudgets for ModelBudgetLimiter<Cache> {
    fn within_model_budget(
        &self,
        subject: &BudgetSubject<'_>,
        model_max_budget: &BudgetMap,
        model: &str,
    ) -> impl Future<Output = Result<(), ErrorClass>> + Send {
        subject_within_model_budget(self, subject, Some(model_max_budget), model)
    }
}

// a budget gate denies on every checker error, and only an unexpected one is worth a warning
pub async fn summary_model_within_budget<Budgets>(
    principal: Option<&UserAPIKeyAuth>,
    budgets: Option<&Budgets>,
    summary_model: &str,
) -> Result<bool, SummaryLimitsError<Infallible>>
where
    Budgets: SummaryModelBudgets,
{
    let (Some(auth), Some(checker)) = (principal, budgets) else {
        return Ok(true);
    };
    let fields: LimitFields = serde_json::to_value(auth)
        .and_then(serde_json::from_value)
        .map_err(SummaryLimitsError::Principal)?;
    for (scope, subject, budget) in budget_scopes(&fields) {
        let checked = checker
            .within_model_budget(&subject, budget, summary_model)
            .await;
        let Err(error) = checked else {
            continue;
        };
        if !matches!(error, ErrorClass::BudgetExceeded(_)) {
            tracing::warn!(
                "compact_20260112: unexpected error during {scope} model-budget check for summary_model={summary_model}; denying: {}",
                class_message(&error)
            );
        }
        return Ok(false);
    }
    Ok(true)
}

// key, user, team, then end user, each only with a nonempty budget and an id; an empty-string id still counts
fn budget_scopes(fields: &LimitFields) -> Vec<(&'static str, BudgetSubject<'_>, &BudgetMap)> {
    let key_budget = fields.model_max_budget.as_ref();
    let key = fields.token.as_ref().map(|token| BudgetSubject::Key {
        entity_id: token.expose_secret(),
        key_alias: fields.key_alias.as_deref(),
    });
    let user = fields
        .user_id
        .as_deref()
        .map(|user_id| BudgetSubject::User { user_id });
    // the key's budget goes along even when the key scope itself was skipped
    let team = fields
        .team_id
        .as_deref()
        .map(|team_id| BudgetSubject::Team {
            team_id,
            key_model_max_budget: key_budget,
        });
    let end_user = fields
        .end_user_id
        .as_deref()
        .map(|end_user_id| BudgetSubject::EndUser { end_user_id });
    [
        ("key", key, key_budget),
        ("user", user, fields.user_model_max_budget.as_ref()),
        ("team", team, fields.team_model_max_budget.as_ref()),
        (
            "end-user",
            end_user,
            fields.end_user_model_max_budget.as_ref(),
        ),
    ]
    .into_iter()
    .filter_map(|(scope, subject, budget)| {
        Some((scope, subject?, budget.filter(|limits| !limits.is_empty())?))
    })
    .collect()
}
