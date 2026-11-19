#![cfg(test)]

use std::collections::BTreeSet;
use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_config::sections::end_user_registry::Settings;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde_json::Number;
use tokio::sync::{Mutex, Notify};

use super::cache::RegistryCache as _;
use super::errors::RegistryError;
use super::loader::{RegistrySpec, load_bounded_registry};
use super::predicate::{RESTRICTED_COLUMNS, restricted_end_user_where};
use super::{EndUserRegistry, RESTRICTED_REGISTRY_MAX_SIZE, RegistryGuards};
use crate::auth_cache::keys::{CacheKey, END_USER_RESTRICTED_OVERFLOW};

mod support;

fn spec() -> RegistrySpec<'static> {
    RegistrySpec {
        cache_key: support::CACHE_KEY,
        overflow_sentinel: END_USER_RESTRICTED_OVERFLOW,
        max_size: RESTRICTED_REGISTRY_MAX_SIZE,
    }
}

fn settings(validate: bool, budget: Option<&str>) -> Settings {
    Settings {
        validate_end_user_id_in_db: validate,
        max_end_user_budget_id: budget.map(str::to_owned),
    }
}

fn guards_none() -> RegistryGuards<'static> {
    RegistryGuards {
        token_end_user_max_budget: None,
        key_end_user_budget_id: None,
    }
}

fn ids(values: &[&str]) -> BTreeSet<String> {
    values.iter().copied().map(str::to_owned).collect()
}

fn sentinel() -> Json {
    Json::Text(END_USER_RESTRICTED_OVERFLOW.to_owned())
}

fn object(entries: [(&str, Json); 1]) -> Json {
    let (name, value) = entries.into_iter().next().expect("one entry");
    Json::Object(IndexMap::from([(name.to_owned(), value)]))
}

fn not_null() -> Json {
    object([("not", Json::Null)])
}

fn cached_list(values: &[&str]) -> Json {
    Json::List(
        values
            .iter()
            .map(|value| Json::Text((*value).to_owned()))
            .collect(),
    )
}

async fn seed_cache(cache: &support::TestCache, answer: Json) {
    cache
        .write(support::CACHE_KEY, &answer, 45.0)
        .await
        .expect("the seeded answer");
}

fn no_database() -> PostgresStore {
    PostgresStore { pool: None }
}

#[tokio::test]
async fn each_restricted_column_nonnull() {
    let expected = Json::Object(IndexMap::from([(
        "OR".to_owned(),
        Json::List(vec![
            object([("blocked", Json::Bool(true))]),
            object([("budget_id", not_null())]),
            object([("allowed_model_region", not_null())]),
            object([("default_model", not_null())]),
            object([("object_permission_id", not_null())]),
        ]),
    )]));
    assert_eq!(restricted_end_user_where(), expected);
    assert_eq!(
        RESTRICTED_COLUMNS,
        [
            "budget_id",
            "allowed_model_region",
            "default_model",
            "object_permission_id"
        ]
    );
    // the SQL behind the same predicate must agree, including the empty string that still is not null
    let (fixture, pool) = support::database().await;
    support::seed(&pool).await;
    let store = support::store(&pool);
    let clock = support::Clock::new(0);
    let cache = support::test_cache(None, &clock);
    let registry = EndUserRegistry::new(settings(false, None));
    let loaded = registry
        .restricted_registry(&store, &cache)
        .await
        .expect("the registry loads");
    assert_eq!(loaded, Some(support::restricted_ids()));
    support::close(fixture, pool).await;
}

#[tokio::test]
async fn blocked_true_only() {
    // blocked restricts on true, never on being set: a false flag is not an alternative in the filter
    let where_clause = restricted_end_user_where();
    let alternatives = match where_clause {
        Json::Object(mut fields) => fields.shift_remove("OR").expect("an OR filter"),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("the restricted filter is an object")
        }
    };
    let Json::List(items) = alternatives else {
        panic!("the restricted filter ORs its alternatives")
    };
    assert_eq!(
        items.first(),
        Some(&object([("blocked", Json::Bool(true))]))
    );
    assert!(!items.contains(&object([("blocked", not_null())])));
    let (fixture, pool) = support::database().await;
    support::seed(&pool).await;
    let store = support::store(&pool);
    let clock = support::Clock::new(0);
    let cache = support::test_cache(None, &clock);
    let registry = EndUserRegistry::new(settings(false, None));
    let loaded = registry
        .restricted_registry(&store, &cache)
        .await
        .expect("the registry loads")
        .expect("the registry is usable");
    assert!(loaded.contains("blocked-user"));
    assert!(!loaded.contains("plain-user"));
    support::close(fixture, pool).await;
}

#[tokio::test]
async fn cache_tuple_and_redis_list() {
    let clock = support::Clock::new(0);
    let cache = support::test_cache(None, &clock);
    // a list stored by the Redis tier, duplicates included, reads back as the same collapsed id set
    seed_cache(&cache, cached_list(&["dup", "dup", "single"])).await;
    let fetches = AtomicUsize::new(0);
    let load_lock = Mutex::new(());
    let loaded = load_bounded_registry(&cache, spec(), &load_lock, || async {
        fetches.fetch_add(1, Ordering::SeqCst);
        Ok::<_, Infallible>(Vec::new())
    })
    .await
    .expect("the cached registry reads");
    assert_eq!(loaded, Some(ids(&["dup", "single"])));
    assert_eq!(fetches.load(Ordering::SeqCst), 0);
    // a fresh load stores the list the memory tier hands back
    let fresh = support::test_cache(None, &clock);
    let fresh_lock = Mutex::new(());
    let fresh_loaded = load_bounded_registry(&fresh, spec(), &fresh_lock, || async {
        Ok::<_, Infallible>(vec!["one".to_owned(), "two".to_owned()])
    })
    .await
    .expect("the registry loads");
    assert_eq!(fresh_loaded, Some(ids(&["one", "two"])));
    assert_eq!(
        fresh.async_get(support::CACHE_KEY, false).await,
        Some(cached_list(&["one", "two"]))
    );
}

#[tokio::test]
async fn nonstring_entries_dropped() {
    let clock = support::Clock::new(0);
    let cache = support::test_cache(None, &clock);
    let malformed = Json::List(vec![
        Json::Text("kept".to_owned()),
        Json::Number(Number::from(1_i64)),
        Json::Bool(true),
        Json::Null,
        Json::List(vec![Json::Text("nested".to_owned())]),
        Json::Object(IndexMap::new()),
    ]);
    seed_cache(&cache, malformed).await;
    let fetches = AtomicUsize::new(0);
    let load_lock = Mutex::new(());
    let loaded = load_bounded_registry(&cache, spec(), &load_lock, || async {
        fetches.fetch_add(1, Ordering::SeqCst);
        Ok::<_, Infallible>(Vec::new())
    })
    .await
    .expect("the cached registry reads");
    assert_eq!(loaded, Some(ids(&["kept"])));
    assert_eq!(fetches.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn empty_registry_usable() {
    let clock = support::Clock::new(0);
    let cache = support::test_cache(None, &clock);
    seed_cache(&cache, Json::List(Vec::new())).await;
    let fetches = AtomicUsize::new(0);
    let load_lock = Mutex::new(());
    let loaded = load_bounded_registry(&cache, spec(), &load_lock, || async {
        fetches.fetch_add(1, Ordering::SeqCst);
        Ok::<_, Infallible>(Vec::new())
    })
    .await
    .expect("the cached registry reads");
    assert_eq!(loaded, Some(BTreeSet::new()));
    assert_eq!(fetches.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn sentinel_unusable() {
    let clock = support::Clock::new(0);
    let cache = support::test_cache(None, &clock);
    seed_cache(&cache, sentinel()).await;
    let fetches = AtomicUsize::new(0);
    let load_lock = Mutex::new(());
    let loaded = load_bounded_registry(&cache, spec(), &load_lock, || async {
        fetches.fetch_add(1, Ordering::SeqCst);
        Ok::<_, Infallible>(vec!["unexpected".to_owned()])
    })
    .await
    .expect("the sentinel reads");
    assert_eq!(loaded, None);
    assert_eq!(fetches.load(Ordering::SeqCst), 0);
    // an unusable registry never proves access, so the caller must look the id up
    let registry = EndUserRegistry::new(settings(false, None));
    let known = registry
        .is_known_unrestricted(&no_database(), &cache, "free-user", guards_none())
        .await
        .expect("the decision reads the sentinel");
    assert!(!known);
}

#[tokio::test]
async fn malformed_cache_miss() {
    let clock = support::Clock::new(0);
    let cache = support::test_cache(None, &clock);
    seed_cache(&cache, Json::Text("not the sentinel".to_owned())).await;
    let fetches = AtomicUsize::new(0);
    let load_lock = Mutex::new(());
    let loaded = load_bounded_registry(&cache, spec(), &load_lock, || async {
        fetches.fetch_add(1, Ordering::SeqCst);
        Ok::<_, Infallible>(vec!["fetched".to_owned()])
    })
    .await
    .expect("the registry loads");
    assert_eq!(loaded, Some(ids(&["fetched"])));
    assert_eq!(fetches.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn unrestricted_registry_hit() {
    let clock = support::Clock::new(0);
    let cache = support::test_cache(None, &clock);
    seed_cache(&cache, cached_list(&["restricted"])).await;
    let registry = EndUserRegistry::new(settings(false, None));
    let store = no_database();
    let free = registry
        .is_known_unrestricted(&store, &cache, "free-user", guards_none())
        .await
        .expect("the free id answers from the registry");
    assert!(free);
    let restricted = registry
        .is_known_unrestricted(&store, &cache, "restricted", guards_none())
        .await
        .expect("the restricted id answers from the registry");
    assert!(!restricted);
}

#[tokio::test]
async fn exact_capacity_fits() {
    let clock = support::Clock::new(0);
    let cache = support::test_cache(None, &clock);
    let load_lock = Mutex::new(());
    let requested = (0..RESTRICTED_REGISTRY_MAX_SIZE)
        .map(|index| format!("id-{index}"))
        .collect::<Vec<_>>();
    let loaded = load_bounded_registry(&cache, spec(), &load_lock, || async {
        Ok::<_, Infallible>(requested)
    })
    .await
    .expect("the registry loads");
    let registry = loaded.expect("exactly the bound is usable");
    assert_eq!(registry.len(), RESTRICTED_REGISTRY_MAX_SIZE);
}

#[tokio::test]
async fn registry_overflow_fallback() {
    let clock = support::Clock::new(0);
    let cache = support::test_cache(Some(60.0_f64), &clock);
    let load_lock = Mutex::new(());
    let fetches = AtomicUsize::new(0);
    let oversized = (0..=RESTRICTED_REGISTRY_MAX_SIZE)
        .map(|index| format!("id-{index}"))
        .collect::<Vec<_>>();
    let (loaded, logs) = support::capture_async(load_bounded_registry(
        &cache,
        spec(),
        &load_lock,
        || async {
            fetches.fetch_add(1, Ordering::SeqCst);
            Ok::<_, Infallible>(oversized)
        },
    ))
    .await;
    assert_eq!(loaded.expect("the overflow reads"), None);
    assert_eq!(fetches.load(Ordering::SeqCst), 1);
    assert!(logs.is_empty(), "the overflow path is silent: {logs}");
    assert_eq!(
        cache.async_get(support::CACHE_KEY, false).await,
        Some(sentinel())
    );
    // the sentinel lives for the management ttl, not the 30 second error ttl
    clock.advance(31);
    let still_unusable = load_bounded_registry(&cache, spec(), &load_lock, || async {
        fetches.fetch_add(1, Ordering::SeqCst);
        Ok::<_, Infallible>(vec!["unexpected".to_owned()])
    })
    .await
    .expect("the sentinel reads");
    assert_eq!(still_unusable, None);
    assert_eq!(fetches.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn overflow_sentinel_expires_at_management_ttl() {
    let clock = support::Clock::new(0);
    let cache = support::test_cache(Some(60.0_f64), &clock);
    let load_lock = Mutex::new(());
    let fetches = AtomicUsize::new(0);
    let oversized = (0..=RESTRICTED_REGISTRY_MAX_SIZE)
        .map(|index| format!("id-{index}"))
        .collect::<Vec<_>>();
    let overflowed = load_bounded_registry(&cache, spec(), &load_lock, || async {
        Ok::<_, Infallible>(oversized)
    })
    .await
    .expect("the overflow reads");
    assert_eq!(overflowed, None);
    // the sentinel expires with the management ttl, so a later load queries the database again
    clock.advance(61);
    let reloaded = load_bounded_registry(&cache, spec(), &load_lock, || async {
        fetches.fetch_add(1, Ordering::SeqCst);
        Ok::<_, Infallible>(vec!["restricted".to_owned()])
    })
    .await
    .expect("the registry reloads");
    assert_eq!(reloaded, Some(ids(&["restricted"])));
    assert_eq!(fetches.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn database_error_warns_thirty_seconds() {
    let clock = support::Clock::new(0);
    let cache = support::test_cache(Some(60.0_f64), &clock);
    let load_lock = Mutex::new(());
    let fetches = AtomicUsize::new(0);
    let (loaded, logs) = support::capture_async(load_bounded_registry(
        &cache,
        spec(),
        &load_lock,
        || async {
            fetches.fetch_add(1, Ordering::SeqCst);
            Err::<Vec<String>, &'static str>("connection refused")
        },
    ))
    .await;
    assert_eq!(loaded.expect("a failed load degrades"), None);
    assert!(logs.contains("per-id lookups will run"), "{logs}");
    assert!(logs.contains("30"), "{logs}");
    assert_eq!(
        cache.async_get(support::CACHE_KEY, false).await,
        Some(sentinel())
    );
    // exactly the error ttl: still suppressed at 30 seconds, queried again past it
    clock.advance(30);
    let still_unusable = load_bounded_registry(&cache, spec(), &load_lock, || async {
        fetches.fetch_add(1, Ordering::SeqCst);
        Ok::<_, Infallible>(vec!["unexpected".to_owned()])
    })
    .await
    .expect("the sentinel reads");
    assert_eq!(still_unusable, None);
    assert_eq!(fetches.load(Ordering::SeqCst), 1);
    clock.advance(1);
    let retried = load_bounded_registry(&cache, spec(), &load_lock, || async {
        fetches.fetch_add(1, Ordering::SeqCst);
        Ok::<_, Infallible>(vec!["restricted".to_owned()])
    })
    .await
    .expect("the registry retries");
    assert_eq!(retried, Some(ids(&["restricted"])));
    assert_eq!(fetches.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn cache_write_warning_preserves_result() {
    let cache = support::FailingCache(support::Failure::Write);
    let load_lock = Mutex::new(());
    let (loaded, logs) = support::capture_async(load_bounded_registry(
        &cache,
        spec(),
        &load_lock,
        || async { Ok::<_, Infallible>(vec!["restricted".to_owned()]) },
    ))
    .await;
    assert_eq!(
        loaded.expect("a failed write keeps the answer"),
        Some(ids(&["restricted"]))
    );
    assert!(logs.contains("Failed to cache registry"), "{logs}");
}

#[tokio::test]
async fn cache_read_error_propagates() {
    let cache = support::FailingCache(support::Failure::Read);
    let load_lock = Mutex::new(());
    let loaded = load_bounded_registry(&cache, spec(), &load_lock, || async {
        Ok::<_, Infallible>(vec!["restricted".to_owned()])
    })
    .await;
    assert!(matches!(loaded, Err(RegistryError::CacheRead(_))));
}

#[tokio::test]
async fn singleflight_waiter_rechecks() {
    let clock = support::Clock::new(0);
    let cache = support::test_cache(None, &clock);
    let load_lock = Mutex::new(());
    let gate = Arc::new(Notify::new());
    let fetches = Arc::new(AtomicUsize::new(0));
    let first_gate = Arc::clone(&gate);
    let first_fetches = Arc::clone(&fetches);
    let first_load = load_bounded_registry(&cache, spec(), &load_lock, move || async move {
        first_fetches.fetch_add(1, Ordering::SeqCst);
        first_gate.notified().await;
        Ok::<_, Infallible>(vec!["restricted".to_owned()])
    });
    let second_fetches = Arc::clone(&fetches);
    let second_load = load_bounded_registry(&cache, spec(), &load_lock, move || async move {
        second_fetches.fetch_add(1, Ordering::SeqCst);
        Ok::<_, Infallible>(vec!["other".to_owned()])
    });
    let notify_first = async {
        gate.notify_one();
    };
    let (first, (), second) = tokio::join!(first_load, notify_first, second_load);
    assert_eq!(first.expect("the first load"), Some(ids(&["restricted"])));
    assert_eq!(
        second.expect("the waiting caller"),
        Some(ids(&["restricted"]))
    );
    assert_eq!(fetches.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn failed_cache_write_waiter_reloads() {
    let gate = Arc::new(Notify::new());
    let fetches = Arc::new(AtomicUsize::new(0));
    let cache = support::FailingCache(support::Failure::Write);
    let load_lock = Mutex::new(());
    let first_gate = Arc::clone(&gate);
    let first_fetches = Arc::clone(&fetches);
    let first_load = load_bounded_registry(&cache, spec(), &load_lock, move || async move {
        first_fetches.fetch_add(1, Ordering::SeqCst);
        first_gate.notified().await;
        Ok::<_, Infallible>(vec!["first".to_owned()])
    });
    let second_fetches = Arc::clone(&fetches);
    let second_load = load_bounded_registry(&cache, spec(), &load_lock, move || async move {
        second_fetches.fetch_add(1, Ordering::SeqCst);
        Ok::<_, Infallible>(vec!["second".to_owned()])
    });
    let notify_first = async {
        gate.notify_one();
    };
    let (first, (), second) = tokio::join!(first_load, notify_first, second_load);
    assert_eq!(first.expect("the first load"), Some(ids(&["first"])));
    assert_eq!(second.expect("the waiting caller"), Some(ids(&["second"])));
    assert_eq!(fetches.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn max_budget_id_disables() {
    let registry = EndUserRegistry::new(settings(false, Some("budget-1")));
    let cache = support::FailingCache(support::Failure::Read);
    let known = registry
        .is_known_unrestricted(&no_database(), &cache, "free-user", guards_none())
        .await
        .expect("the skip is decided before the cache");
    assert!(!known);
}

#[tokio::test]
async fn empty_budget_id_disables() {
    let registry = EndUserRegistry::new(settings(false, Some("")));
    let cache = support::FailingCache(support::Failure::Read);
    let known = registry
        .is_known_unrestricted(&no_database(), &cache, "free-user", guards_none())
        .await
        .expect("the skip is decided before the cache");
    assert!(!known);
}

#[tokio::test]
async fn key_budget_disables() {
    let registry = EndUserRegistry::new(settings(false, None));
    let cache = support::FailingCache(support::Failure::Read);
    let guards = RegistryGuards {
        token_end_user_max_budget: None,
        key_end_user_budget_id: Some(""),
    };
    let known = registry
        .is_known_unrestricted(&no_database(), &cache, "free-user", guards)
        .await
        .expect("the skip is decided before the cache");
    assert!(!known);
}

#[tokio::test]
async fn validation_disables() {
    let registry = EndUserRegistry::new(settings(true, None));
    let cache = support::FailingCache(support::Failure::Read);
    let known = registry
        .is_known_unrestricted(&no_database(), &cache, "free-user", guards_none())
        .await
        .expect("the skip is decided before the cache");
    assert!(!known);
}

#[tokio::test]
async fn token_zero_budget_disables() {
    let registry = EndUserRegistry::new(settings(false, None));
    let cache = support::FailingCache(support::Failure::Read);
    let guards = RegistryGuards {
        token_end_user_max_budget: Some(Decimal::ZERO),
        key_end_user_budget_id: None,
    };
    let known = registry
        .is_known_unrestricted(&no_database(), &cache, "free-user", guards)
        .await
        .expect("the skip is decided before the cache");
    assert!(!known);
}

#[tokio::test]
async fn registry_key_literal() {
    assert_eq!(
        CacheKey::EndUserRestrictedRegistry.render(),
        support::CACHE_KEY
    );
    let clock = support::Clock::new(0);
    let cache = support::test_cache(None, &clock);
    seed_cache(&cache, cached_list(&["restricted"])).await;
    let registry = EndUserRegistry::new(settings(false, None));
    let known = registry
        .is_known_unrestricted(&no_database(), &cache, "free-user", guards_none())
        .await
        .expect("the decision reads the registry key");
    assert!(known);
}

#[tokio::test]
async fn restricted_id_requires_lookup() {
    let (fixture, pool) = support::database().await;
    support::seed(&pool).await;
    let store = support::store(&pool);
    let clock = support::Clock::new(0);
    let cache = support::test_cache(None, &clock);
    let registry = EndUserRegistry::new(settings(false, None));
    let free = registry
        .is_known_unrestricted(&store, &cache, "plain-user", guards_none())
        .await
        .expect("the unrestricted id answers");
    assert!(free);
    // the restricted id is in the registry, so only a per-id lookup can decide its access
    let restricted = registry
        .is_known_unrestricted(&store, &cache, "budget-user", guards_none())
        .await
        .expect("the restricted id answers");
    assert!(!restricted);
    support::close(fixture, pool).await;
}
