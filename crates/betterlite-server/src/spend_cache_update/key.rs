use std::sync::Arc;

use betterlite_auth::resolved_key::view::VerificationTokenView;
use betterlite_db::key_user_writes::hash::hash_token_if_needed;
use rust_decimal::Decimal;
use secrecy::ExposeSecret as _;

use crate::failure_hook::ports::{CallInfo, EntityType};
use crate::proxy_helpers::projected::{
    ProjectedLimit, is_projected_spend_over_limit, projected_spend_over_limit,
};

use super::{SpendCacheUpdateError, UpdateDeps, added_spend};

// the key object is only read here: LiteLLM never queues it for a write, only the projected alert leaves this branch
pub(super) async fn update_key_spend(
    token: &str,
    response_cost: Decimal,
    deps: &UpdateDeps,
) -> Result<(), SpendCacheUpdateError> {
    let hashed = hash_token_if_needed(token);
    let Some(existing) = deps.cache.auth(&hashed).await? else {
        return Ok(());
    };
    let serialized = serde_json::to_value(&existing).map_err(SpendCacheUpdateError::KeyView)?;
    let view: VerificationTokenView =
        serde_json::from_value(serialized).map_err(SpendCacheUpdateError::KeyView)?;
    let new_spend = added_spend(view.token.spend, response_cost)?;
    let Some(projected) = projected_over_soft_budget(&view, new_spend, deps)? else {
        return Ok(());
    };
    let info = projected_call_info(&view, new_spend, &projected);
    let alerts = Arc::clone(&deps.alerts);
    deps.tasks.spawn(async move {
        alerts.projected_limit_exceeded(info).await;
    });
    Ok(())
}

fn projected_over_soft_budget(
    view: &VerificationTokenView,
    new_spend: Decimal,
    deps: &UpdateDeps,
) -> Result<Option<ProjectedLimit>, SpendCacheUpdateError> {
    let soft_budget = view.soft_budget;
    if view.token.soft_budget_cooldown.0 || soft_budget.is_none() {
        return Ok(None);
    }
    let today = deps.clock.now().date_naive();
    let over_limit = is_projected_spend_over_limit(new_spend, soft_budget, today)
        .map_err(SpendCacheUpdateError::Projection)?;
    if !over_limit {
        return Ok(None);
    }
    projected_spend_over_limit(new_spend, soft_budget, today)
        .map_err(SpendCacheUpdateError::Projection)
}

fn projected_call_info(
    view: &VerificationTokenView,
    new_spend: Decimal,
    projected: &ProjectedLimit,
) -> CallInfo {
    CallInfo {
        spend: new_spend,
        max_budget: view.soft_budget,
        soft_budget: None,
        token: Some(
            view.token
                .token
                .as_ref()
                .map_or_else(String::new, |secret| secret.expose_secret().to_owned()),
        ),
        customer_id: None,
        user_id: view.token.user_id.clone(),
        team_id: None,
        team_alias: None,
        organization_id: None,
        user_email: None,
        key_alias: view.token.key_alias.clone(),
        projected_exceeded_date: Some(projected.limit_exceed_date.to_string()),
        projected_spend: Some(projected.projected_spend),
        event_group: EntityType::Key,
        alert_emails: None,
        max_budget_alert_emails: None,
    }
}
