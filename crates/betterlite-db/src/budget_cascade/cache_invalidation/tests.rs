#![cfg(test)]

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use super::commit_and_invalidate_cascade;
use crate::budget_cascade::tests::{
    Database, RecordingCache, cascade, reject_budget_updates, seed_linked_rows, spend_of,
};
use crate::budget_cascade::{BudgetCascade, BudgetReset, CounterReset};
use crate::budget_reset_policy::cache_invalidation::{CacheDeleteFuture, ResetCache};
use crate::postgres_store::session::PostgresStore;

fn collected_with_cache_entries() -> BudgetCascade {
    let mut collected = cascade(&["budget-a"], BTreeMap::new());
    collected.counter_resets.push(CounterReset {
        counter_key: "spend:org:org-a".to_owned(),
        carried_spend: Decimal::ZERO,
    });
    collected.cache_keys.push("org_id:org-a".to_owned());
    collected.budget_resets.push(BudgetReset {
        budget_id: "budget-a".to_owned(),
        reset_at: DateTime::<Utc>::from_timestamp(1_700_000_000, 0).expect("timestamp is valid"),
    });
    collected
}

#[tokio::test]
async fn counters_drop_after_the_rows_commit() {
    let database = Database::provision().await;
    seed_linked_rows(&database.client).await;
    let cache = RecordingCache::probing(database.store.clone());
    commit_and_invalidate_cascade(
        &database.store,
        &cache,
        &collected_with_cache_entries(),
        None,
    )
    .await
    .expect("cascade commits");
    assert_eq!(cache.counters(), vec![vec!["spend:org:org-a".to_owned()]]);
    assert_eq!(cache.keys(), vec![vec!["org_id:org-a".to_owned()]]);
    assert_eq!(cache.observed_spend(), vec![Decimal::ZERO]);
    database.finish().await;
}

#[tokio::test]
async fn a_failed_commit_drops_no_cache_entry() {
    let database = Database::provision().await;
    seed_linked_rows(&database.client).await;
    reject_budget_updates(&database.client).await;
    let cache = RecordingCache::silent();
    let outcome = commit_and_invalidate_cascade(
        &database.store,
        &cache,
        &collected_with_cache_entries(),
        None,
    )
    .await;
    assert!(outcome.is_err());
    assert!(cache.counters().is_empty());
    assert!(cache.keys().is_empty());
    database.finish().await;
}

#[tokio::test]
async fn cache_failures_do_not_undo_the_commit() {
    let database = Database::provision().await;
    seed_linked_rows(&database.client).await;
    let outcome = commit_and_invalidate_cascade(
        &database.store,
        &FailingCache,
        &collected_with_cache_entries(),
        None,
    )
    .await;
    outcome.expect("a failed cache invalidation still commits the reset");
    assert_eq!(
        spend_of(
            &database.client,
            "LiteLLM_OrganizationTable",
            "organization_id",
            "org-a"
        )
        .await,
        Decimal::ZERO
    );
    database.finish().await;
}

#[tokio::test]
async fn an_empty_cascade_invalidates_nothing() {
    let store = PostgresStore { pool: None };
    let cache = RecordingCache::silent();
    commit_and_invalidate_cascade(&store, &cache, &BudgetCascade::default(), None)
        .await
        .expect("nothing to commit");
    assert!(cache.counters().is_empty());
    assert!(cache.keys().is_empty());
}

#[derive(Debug, thiserror::Error)]
#[error("cache backend unavailable")]
struct CacheError;

struct FailingCache;

impl ResetCache for FailingCache {
    type Failure = CacheError;

    fn delete_spend_counters<'cache>(
        &'cache self,
        _keys: &'cache [String],
    ) -> CacheDeleteFuture<'cache, CacheError> {
        Box::pin(async move { Err(CacheError) })
    }

    fn delete_user_api_keys<'cache>(
        &'cache self,
        _keys: &'cache [String],
    ) -> CacheDeleteFuture<'cache, CacheError> {
        Box::pin(async move { Err(CacheError) })
    }
}
