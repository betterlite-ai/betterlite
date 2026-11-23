use std::cmp::Ordering;

use betterlite_models::baseline_history::model::BaselineHistory;
use betterlite_models::savings_pricing::context::SavingsPricingContext;
use deadpool_postgres::Transaction;
use tokio::time::timeout;
use uuid::Uuid;

use super::error::StoreError;
use super::pages::{PageQuery, lock_comparison, read_page};
use super::sql::{CHANGED_BEFORE, MISSING_LOG, STORE_HISTORY};
use super::{
    ACCOUNTING_LIMITS, BaselineAccountingStore, ProjectResult, TRANSACTION_TIMEOUT, begin,
};
use crate::baseline::publication::project::project_group;
use crate::baseline::publication::publish::publish;
use crate::database_replica::pinned::writer_wrapper;

// A failed projection rolls every page back, so the comparison stays dirty for the next claim.
pub async fn project_scope<C, G>(
    store: &BaselineAccountingStore<'_, C, G>,
    scope: &str,
) -> ProjectResult
where
    C: SavingsPricingContext + Sync,
    G: Fn() -> Uuid + Sync,
{
    let projected = async {
        let mut session = writer_wrapper(store.database).session().await?;
        let transaction = begin(&mut session, ACCOUNTING_LIMITS).await?;
        let outcome = project_rows(&transaction, store, scope).await?;
        transaction.commit().await?;
        Ok::<_, StoreError>(outcome)
    };
    let settled = timeout(TRANSACTION_TIMEOUT, projected)
        .await
        .unwrap_or_else(|elapsed| Err(StoreError::from(elapsed)));
    settled.unwrap_or_else(|error| {
        tracing::warn!(%error, "Auto-router baseline projection remains pending");
        ProjectResult::Unavailable
    })
}

async fn project_rows<C, G>(
    transaction: &Transaction<'_>,
    store: &BaselineAccountingStore<'_, C, G>,
    scope: &str,
) -> Result<ProjectResult, StoreError>
where
    C: SavingsPricingContext + Sync,
    G: Fn() -> Uuid + Sync,
{
    let Some(state) = lock_comparison(transaction, scope).await? else {
        return Ok(ProjectResult::Unchanged);
    };
    if state.retired || state.revision == state.published_revision {
        return Ok(ProjectResult::Unchanged);
    }
    if transaction
        .query_opt(MISSING_LOG, &[&scope])
        .await?
        .is_some()
    {
        return Ok(ProjectResult::Unavailable);
    }
    let initial = BaselineHistory {
        first_at: None,
        last_at: None,
        equivalent: state.initial_equivalent,
        uncertain_before: 0.0_f64,
        entries: Vec::new(),
        blocked_until: 0.0_f64,
    };
    let checkpoint = match &state.history {
        Some(text) => serde_json::from_str(text)?,
        None => initial.clone(),
    };
    let late = transaction
        .query_opt(
            CHANGED_BEFORE,
            &[&scope, &state.published_revision, &checkpoint.last_at],
        )
        .await?;
    // A newer observation at or before the checkpoint changes what it folded, so the replay restarts from scratch.
    let (start, after_revision) = if late.is_some() {
        (initial, 0)
    } else {
        (checkpoint, state.published_revision)
    };
    let query = PageQuery {
        scope,
        after_revision,
        withdraw_from: None,
    };
    let history = replay(transaction, store, &query, start).await?;
    let stored = serde_json::to_string(&history)?;
    transaction
        .execute(STORE_HISTORY, &[&scope, &stored])
        .await?;
    Ok(ProjectResult::Published)
}

// Each page folds its complete start-time groups and publishes their corrections before the next page reads.
async fn replay<C, G>(
    transaction: &Transaction<'_>,
    store: &BaselineAccountingStore<'_, C, G>,
    query: &PageQuery<'_>,
    start: BaselineHistory,
) -> Result<BaselineHistory, StoreError>
where
    C: SavingsPricingContext + Sync,
    G: Fn() -> Uuid + Sync,
{
    let mut history = start;
    let mut cursor = None;
    loop {
        let page = read_page(transaction, query, cursor).await?;
        let Some(last) = page.last() else {
            return Ok(history);
        };
        cursor = Some(last.started_at);
        let mut projected = (history, Vec::new());
        let groups = page.chunk_by(|left, right| {
            left.started_at
                .partial_cmp(&right.started_at)
                .is_some_and(Ordering::is_eq)
        });
        for group in groups {
            projected = project_group(&store.pricing, &projected, group)?;
        }
        publish(transaction, &projected.1, &store.generate_id).await?;
        history = projected.0;
    }
}
