use betterlite_db::identity_rows::end_user::LiteLLMEndUserTable;
use betterlite_db::spend_rows::key::END_USER_COUNTER_PREFIX;
use betterlite_db::spend_rows::rows::SpendCounterRows as DbSpendCounterRows;
use betterlite_error::error::details::BudgetExceededDetails;

use crate::auxiliary_routes::INFO_ROUTES;
use crate::current_spend::{CurrentSpend, CurrentSpendError, SpendQuery, WindowDescriptors};
use crate::spend_reads::batch::SpendBatch;
use crate::spend_reads::cache::SpendReadCache;
use crate::spend_reseed::rows::SpendCounterRows;

// litellm.budget_exceeded_status_code: the default the ProxyException carries
pub const BUDGET_EXCEEDED_STATUS: u16 = 422;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{}", .details.message.as_deref().unwrap_or("end-user budget exceeded"))]
pub struct EndUserBudgetExceeded {
    pub details: BudgetExceededDetails,
}

// The spend failure keeps its own cause so the stage can answer 401 with LiteLLM's exception text
#[derive(Debug, thiserror::Error)]
pub enum EndUserBudgetError<CacheError> {
    #[error("the end-user spend could not be read")]
    Spend(#[source] CurrentSpendError<CacheError>),
    #[error(transparent)]
    Exceeded(#[from] EndUserBudgetExceeded),
}

// LiteLLM's _check_end_user_budget: info routes, an absent relation or ceiling, then B2321's spend read
pub async fn check_end_user_budget<Cache, Batch, Rows>(
    end_user: &LiteLLMEndUserTable,
    route: &str,
    spend: &CurrentSpend<'_, Cache, Batch, Rows>,
) -> Result<(), EndUserBudgetError<Cache::Error>>
where
    Cache: SpendReadCache + ?Sized,
    Batch: SpendBatch + ?Sized,
    Rows: SpendCounterRows + DbSpendCounterRows,
{
    if INFO_ROUTES.contains(&route) {
        return Ok(());
    }
    let Some(budget) = end_user.litellm_budget_table.as_ref() else {
        return Ok(());
    };
    let Some(max_budget) = budget.max_budget else {
        return Ok(());
    };
    let counter_key = format!("{END_USER_COUNTER_PREFIX}{}", end_user.user_id);
    let query = SpendQuery {
        counter_key: &counter_key,
        fallback_spend: end_user.spend,
        max_budget: Some(max_budget),
        window: WindowDescriptors {
            entity_type: None,
            entity_id: None,
            duration: None,
            start: None,
        },
        fallback_authoritative: true,
    };
    let current = spend
        .get_current_spend(&query)
        .await
        .map_err(EndUserBudgetError::Spend)?;
    if current <= max_budget {
        return Ok(());
    }
    Err(EndUserBudgetError::from(EndUserBudgetExceeded {
        details: BudgetExceededDetails {
            current_cost: current,
            max_budget,
            message: Some(format!(
                "ExceededBudget: End User={0} over budget. Spend={1}, Budget={2}",
                end_user.user_id, current, max_budget
            )),
            provider: String::new(),
            entity_type: Some("end_user".to_owned()),
            entity_id: Some(end_user.user_id.clone()),
            status_code: BUDGET_EXCEEDED_STATUS,
        },
    }))
}
