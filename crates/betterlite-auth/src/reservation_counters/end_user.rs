use betterlite_db::identity_rows::end_user::LiteLLMEndUserTable;
use rust_decimal::Decimal;

use crate::auth_cache::keys::CacheKey;
use crate::budget_counter::{BudgetCounter, CounterEntityType};
use crate::resolved_key::auth::UserAPIKeyAuth;

pub(crate) fn end_user_counter(
    token: &UserAPIKeyAuth,
    end_user_id: Option<&str>,
    end_user: Option<&LiteLLMEndUserTable>,
) -> Option<BudgetCounter> {
    let resolved_id = end_user_id.or(token.view.end_user_id.as_deref())?;
    let mut budget_cap = token.view.end_user_max_budget;
    let mut fallback_spend = Decimal::ZERO;
    if let Some(object) = end_user {
        fallback_spend = object.spend;
        if budget_cap.is_none() {
            budget_cap = object
                .litellm_budget_table
                .as_ref()
                .and_then(|budget| budget.max_budget);
        }
    }
    let max_budget = budget_cap.filter(|value| *value > Decimal::ZERO)?;
    Some(BudgetCounter {
        counter_key: format!("spend:end_user:{resolved_id}"),
        max_budget,
        fallback_spend,
        entity_type: CounterEntityType::EndUser,
        entity_id: resolved_id.to_owned(),
        source_cache_key: Some(CacheKey::EndUserId(resolved_id).render()),
        spend_log_entity_id: None,
        window_duration: None,
        window_start: None,
    })
}
