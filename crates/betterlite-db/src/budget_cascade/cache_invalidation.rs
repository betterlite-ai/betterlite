use super::BudgetCascade;
use super::transaction::commit_budget_cascade;
use crate::budget_reset_policy::cache_invalidation::ResetCache;
use crate::budget_reset_policy::invalidation_order::commit_then_invalidate;
use crate::postgres_store::session::PostgresStore;
use crate::spend_reset_unit_of_work::batch::CommitError;

// _invalidate_budget_cascade_caches: the non-enduser counters and cache entries are dropped only after the transaction commits, so a failed write leaves the reservation path reading the still-live spend.
pub async fn commit_and_invalidate_cascade<Cache>(
    store: &PostgresStore,
    cache: &Cache,
    cascade: &BudgetCascade,
    default_enduser_budget_id: Option<&str>,
) -> Result<(), CommitError>
where
    Cache: ResetCache,
{
    let mut counter_keys = Vec::with_capacity(cascade.counter_resets.len());
    for reset in &cascade.counter_resets {
        counter_keys.push(reset.counter_key.clone());
    }
    commit_then_invalidate(
        cache,
        || commit_budget_cascade(store, cascade, default_enduser_budget_id),
        &counter_keys,
        &cascade.cache_keys,
    )
    .await
}

mod tests;
