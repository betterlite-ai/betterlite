use std::sync::Arc;

use betterlite_auth::auth_cache::payload;
use betterlite_db::identity_rows::PydanticBool;
use betterlite_db::key_user_writes::hash::hash_token_if_needed;
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::failure_hook::ports::{CallInfo, EntityType};
use crate::proxy_helpers::projected::{
    ProjectedLimit, is_projected_spend_over_limit, projected_spend_over_limit,
};

use super::{SpendCacheUpdateError, UpdateDeps, added_spend};

// the fields LiteLLM reads off the cached key object; a payload that fails validation reads as a miss
#[derive(Deserialize)]
struct CachedKeyObject {
    #[serde(default)]
    spend: Decimal,
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    key_alias: Option<String>,
    #[serde(default)]
    soft_budget: Option<Decimal>,
    #[serde(default)]
    soft_budget_cooldown: PydanticBool,
}

// the key object is only read here: LiteLLM never queues it for a write, only the projected alert leaves this branch
pub(super) async fn update_key_spend(
    token: &str,
    response_cost: Decimal,
    deps: &UpdateDeps,
) -> Result<(), SpendCacheUpdateError> {
    let hashed = hash_token_if_needed(token);
    let Some(cached) = deps.cache.cached_key(&hashed).await else {
        return Ok(());
    };
    let Some(object) =
        payload::decode::<CachedKeyObject>(&cached).map_err(SpendCacheUpdateError::Decode)?
    else {
        return Ok(());
    };
    let new_spend = added_spend(object.spend, response_cost)?;
    let Some(projected) = projected_over_soft_budget(&object, new_spend, deps)? else {
        return Ok(());
    };
    let info = projected_call_info(&object, new_spend, &projected);
    let alerts = Arc::clone(&deps.alerts);
    deps.tasks.spawn(async move {
        alerts.projected_limit_exceeded(info).await;
    });
    Ok(())
}

fn projected_over_soft_budget(
    object: &CachedKeyObject,
    new_spend: Decimal,
    deps: &UpdateDeps,
) -> Result<Option<ProjectedLimit>, SpendCacheUpdateError> {
    if object.soft_budget_cooldown.0 || object.soft_budget.is_none() {
        return Ok(None);
    }
    let today = deps.clock.now().date_naive();
    let over_limit = is_projected_spend_over_limit(new_spend, object.soft_budget, today)
        .map_err(SpendCacheUpdateError::Projection)?;
    if !over_limit {
        return Ok(None);
    }
    projected_spend_over_limit(new_spend, object.soft_budget, today)
        .map_err(SpendCacheUpdateError::Projection)
}

fn projected_call_info(
    object: &CachedKeyObject,
    new_spend: Decimal,
    projected: &ProjectedLimit,
) -> CallInfo {
    CallInfo {
        spend: new_spend,
        max_budget: object.soft_budget,
        soft_budget: None,
        token: Some(object.token.clone().unwrap_or_default()),
        customer_id: None,
        user_id: object.user_id.clone(),
        team_id: None,
        team_alias: None,
        organization_id: None,
        user_email: None,
        key_alias: object.key_alias.clone(),
        projected_exceeded_date: Some(projected.limit_exceed_date.to_string()),
        projected_spend: Some(projected.projected_spend),
        event_group: EntityType::Key,
        alert_emails: None,
        max_budget_alert_emails: None,
    }
}
