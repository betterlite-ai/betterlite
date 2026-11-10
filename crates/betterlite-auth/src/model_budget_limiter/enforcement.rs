mod tests;

use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_error::error::ErrorClass;
use betterlite_error::error::details::BudgetExceededDetails;
use rust_decimal::Decimal;

use crate::model_budget_values::budget_config::BudgetRuntime;
use crate::model_budget_values::budget_scope::BudgetScope;
use crate::model_budget_values::legacy_carry::LegacyCarry;
use crate::model_budget_values::resolution::{ResolvedModelBudget, resolve_model_budget};
use crate::model_budget_values::spend_keys::model_budget_spend_cache_key;
use crate::model_budget_values::team_override::team_model_budget_applies;

use super::ModelBudgetLimiter;
use super::cache::{ModelBudgetCache, cached_spend};

// the entity whose model budget is checked, with the fields each LiteLLM check takes
pub enum BudgetSubject<'input> {
    Key {
        entity_id: &'input str,
        key_alias: Option<&'input str>,
    },
    User {
        user_id: &'input str,
    },
    EndUser {
        end_user_id: &'input str,
    },
    Team {
        team_id: &'input str,
        key_model_max_budget: Option<&'input BTreeMap<String, JsonData>>,
    },
}

struct EntityCheck<'input> {
    scope: BudgetScope,
    entity_id: &'input str,
    model_max_budget: Option<&'input BTreeMap<String, JsonData>>,
    model: &'input str,
    message: String,
}

pub async fn subject_within_model_budget<C>(
    limiter: &ModelBudgetLimiter<C>,
    subject: &BudgetSubject<'_>,
    model_max_budget: Option<&BTreeMap<String, JsonData>>,
    model: &str,
) -> Result<(), ErrorClass>
where
    C: ModelBudgetCache,
{
    if let BudgetSubject::Team {
        key_model_max_budget,
        ..
    } = subject
    {
        let runtime = BudgetRuntime {
            durations: limiter.durations.as_ref(),
            now: limiter.clock.now(),
        };
        let team_applies = team_model_budget_applies(
            model,
            *key_model_max_budget,
            limiter.catalog.as_ref(),
            runtime,
        );
        if !team_applies {
            return Ok(());
        }
    }
    let (scope, entity_id, message) = check_parts(subject, model);
    entity_within_model_budget(
        limiter,
        EntityCheck {
            scope,
            entity_id,
            model_max_budget,
            model,
            message,
        },
    )
    .await
}

fn check_parts<'input>(
    subject: &BudgetSubject<'input>,
    model: &str,
) -> (BudgetScope, &'input str, String) {
    match subject {
        BudgetSubject::Key {
            entity_id,
            key_alias,
        } => {
            let alias = key_alias.unwrap_or("none");
            let message = format!(
                "virtual key {entity_id} (alias {alias}) exceeded its budget for model {model}"
            );
            (BudgetScope::Key, entity_id, message)
        }
        BudgetSubject::User { user_id } => (
            BudgetScope::User,
            user_id,
            format!("user {user_id} exceeded its budget for model {model}"),
        ),
        BudgetSubject::EndUser { end_user_id } => (
            BudgetScope::EndUser,
            end_user_id,
            format!("end user {end_user_id} exceeded its budget for model {model}"),
        ),
        BudgetSubject::Team { team_id, .. } => (
            BudgetScope::Team,
            team_id,
            format!("team {team_id} exceeded its budget for model {model}"),
        ),
    }
}

async fn entity_within_model_budget<C>(
    limiter: &ModelBudgetLimiter<C>,
    check: EntityCheck<'_>,
) -> Result<(), ErrorClass>
where
    C: ModelBudgetCache,
{
    let Some(budgets) = check.model_max_budget.filter(|budgets| !budgets.is_empty()) else {
        return Ok(());
    };
    let runtime = BudgetRuntime {
        durations: limiter.durations.as_ref(),
        now: limiter.clock.now(),
    };
    let Some(resolved) =
        resolve_model_budget(check.model, budgets, limiter.catalog.as_ref(), runtime)
    else {
        return Ok(());
    };
    let Some(max_budget) = resolved.budget_config.max_budget else {
        return Ok(());
    };
    if max_budget < Decimal::ZERO {
        return Ok(());
    }
    let current_spend = spend_for_model_budget(limiter, &check, &resolved).await;
    if current_spend < max_budget {
        return Ok(());
    }
    Err(ErrorClass::BudgetExceeded(Box::new(
        BudgetExceededDetails {
            current_cost: current_spend,
            max_budget,
            message: Some(check.message),
            provider: String::new(),
            entity_type: Some(entity_type_name(check.scope).to_owned()),
            entity_id: Some(check.entity_id.to_owned()),
            status_code: limiter.budget_exceeded_status,
        },
    )))
}

async fn spend_for_model_budget<C>(
    limiter: &ModelBudgetLimiter<C>,
    check: &EntityCheck<'_>,
    resolved: &ResolvedModelBudget,
) -> Decimal
where
    C: ModelBudgetCache,
{
    let duration = &resolved.budget_config.budget_duration;
    let spend_key = model_budget_spend_cache_key(
        check.scope,
        check.entity_id,
        &resolved.budget_model,
        duration,
    );
    let current = cached_spend(limiter, &spend_key).await;
    let runtime = BudgetRuntime {
        durations: limiter.durations.as_ref(),
        now: limiter.clock.now(),
    };
    let carry = LegacyCarry {
        runtime,
        uptime: limiter.clock.uptime(),
    };
    let Some(legacy_key) =
        carry.request_model_spend_cache_key(check.scope, check.entity_id, check.model, resolved)
    else {
        return current;
    };
    let legacy = cached_spend(limiter, &legacy_key).await;
    current.saturating_add(legacy)
}

const fn entity_type_name(scope: BudgetScope) -> &'static str {
    match scope {
        BudgetScope::Key => "key",
        BudgetScope::User => "user",
        BudgetScope::EndUser => "end_user",
        BudgetScope::Team => "team",
    }
}
