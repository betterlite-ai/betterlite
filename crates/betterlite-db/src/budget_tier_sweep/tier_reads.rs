use chrono::{DateTime, Utc};

use crate::budget_permission_rows::LiteLLMBudgetTableFull;
use crate::budget_reset_policy::retry::RetryMode;
use crate::database_faults::DbError;
use crate::database_faults::ErrorType;
use crate::database_faults::reconnect::{DbReconnect, call_with_db_reconnect_retry};
use crate::identity_budget_reset::faults::{describe, read_fault};
use crate::postgres_store::errors::backend;
use crate::postgres_store::session::PostgresStore;
use crate::table_ops::rows::RowDecoder;

use super::tier_null_reset::DUE_TIERS_SQL;

pub(crate) const TIER_READ_REASON: &str = "reset_budget_read_budgets_failure";

// _with_db_retry: a chunk read replays every transport error, since a read has nothing to double-apply.
pub(crate) async fn read_due_tiers<Reconnect>(
    store: &PostgresStore,
    now: DateTime<Utc>,
    limit: usize,
    reconnect: Option<&mut Reconnect>,
) -> Result<Vec<LiteLLMBudgetTableFull>, DbError>
where
    Reconnect: DbReconnect + Send,
{
    let policy = RetryMode::Read.policy(TIER_READ_REASON);
    let attempt = move || read_tier_page(store, now, limit);
    call_with_db_reconnect_retry(reconnect, attempt, policy).await
}

pub(crate) async fn read_tier_page(
    store: &PostgresStore,
    now: DateTime<Utc>,
    limit: usize,
) -> Result<Vec<LiteLLMBudgetTableFull>, DbError> {
    let client = store.session().await.map_err(|error| read_fault(&error))?;
    let moment = now.naive_utc();
    let bound = i64::try_from(limit).unwrap_or(i64::MAX);
    let rows = client
        .query(DUE_TIERS_SQL, &[&moment, &bound])
        .await
        .map_err(|source| read_fault(&backend(source)))?;
    let mut budgets = Vec::with_capacity(rows.len());
    for row in &rows {
        let text: String = row
            .try_get(0)
            .map_err(|source| read_fault(&backend(source)))?;
        budgets.push(decode_tier(&text)?);
    }
    Ok(budgets)
}

fn decode_tier(text: &str) -> Result<LiteLLMBudgetTableFull, DbError> {
    LiteLLMBudgetTableFull::decode_row(text).map_err(|error| DbError {
        error_type: ErrorType::Other,
        message: describe(&error),
        code: None,
        meta_code: None,
        traceback_modules: Vec::new(),
        cause: None,
    })
}

mod tests;
