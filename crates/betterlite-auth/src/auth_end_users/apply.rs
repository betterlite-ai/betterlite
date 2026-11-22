use betterlite_db::identity_rows::end_user::LiteLLMEndUserTable;

use crate::auth_cache::payload::PayloadError;

use super::cache::EndUserCache;
use super::defaults::DefaultBudgetLookup;
use super::rows::BudgetRows;

// LiteLLM's _apply_default_budget: the row keeps its own budget_id, only a copy changes
pub async fn apply_default_budget<Rows, Cache>(
    end_user: &LiteLLMEndUserTable,
    lookup: &DefaultBudgetLookup<'_, Rows, Cache>,
    key_end_user_budget_id: Option<&str>,
) -> Result<LiteLLMEndUserTable, PayloadError>
where
    Rows: BudgetRows + ?Sized,
    Cache: EndUserCache + ?Sized,
{
    if end_user.budget_id.is_some() && end_user.litellm_budget_table.is_some() {
        return Ok(end_user.clone());
    }
    if key_end_user_budget_id.is_none() && lookup.max_end_user_budget_id.is_none() {
        return Ok(end_user.clone());
    }
    let Some(default_budget) = lookup
        .resolve_default_budget(key_end_user_budget_id)
        .await?
    else {
        return Ok(end_user.clone());
    };
    tracing::debug!(
        "Applied default budget {} to end user {}",
        default_budget.budget_id.as_deref().unwrap_or("None"),
        end_user.user_id
    );
    let mut applied = end_user.clone();
    applied.litellm_budget_table = Some(default_budget);
    Ok(applied)
}
