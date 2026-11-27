#![cfg(test)]

use std::sync::{Arc, Mutex};

use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{DualCacheSettings, MAX_REDIS_BATCH_CACHE_SIZE};
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_models::decisions::Json;
use rstest::rstest;

use super::super::cache::{POSITIVE_TTL_SECONDS, ValidationCache};
use super::super::existence::{
    EndUserExistence as _, EndUserObjectExistence, ExistenceError, UserExistence, UserProbe,
};
use super::super::resolve::{DatabaseLookups, EndUserIdResolver};
use super::support::{
    Answer, RowAnswer, RowCache, ScriptedCache, ScriptedEndUsers, ScriptedRows, ScriptedUserStore,
    ScriptedUsers, TestCache, capture_events, debug_messages, end_user_row, registry, request,
    test_cache, user_lookup,
};
use crate::auth_cache::{WriteTtl, gateway_cache};
use crate::auth_end_users::object::EndUserObjectError;
use crate::auth_user_lookup::cache::UserCache as _;
use crate::auth_user_lookup::lookup::UserLookup;

fn adapter(
    rows: &Arc<ScriptedRows>,
    cache: Arc<RowCache>,
    max_end_user_budget_id: Option<&str>,
) -> EndUserObjectExistence<ScriptedRows, ScriptedRows, RowCache> {
    EndUserObjectExistence {
        rows: Some(Arc::clone(rows)),
        budgets: Some(Arc::clone(rows)),
        cache,
        registry: Arc::new(registry(true, max_end_user_budget_id)),
        store: PostgresStore { pool: None },
        max_end_user_budget_id: max_end_user_budget_id.map(str::to_owned),
    }
}

fn timed_cache(now: &Arc<Mutex<f64>>) -> TestCache {
    let time = Arc::clone(now);
    let memory =
        Arc::new(LocalMemoryOps::new(1024, Arc::new(move || *time.lock().unwrap())).unwrap());
    gateway_cache(
        Arc::clone(&memory),
        memory,
        None,
        DualCacheSettings {
            default_in_memory_ttl: None,
            default_redis_ttl: None,
            batch_cache_expiry: None,
            max_redis_batch_cache_size: MAX_REDIS_BATCH_CACHE_SIZE,
            redis_delete_chunk_size: MAX_REDIS_BATCH_CACHE_SIZE,
        },
        45.0_f64,
    )
}

#[rstest]
#[case::positive(Answer::Found, 299.0_f64, 301.0_f64)]
#[case::negative(Answer::Missing, 59.0_f64, 61.0_f64)]
#[tokio::test]
async fn validation_ttl_expires(#[case] answer: Answer, #[case] before: f64, #[case] after: f64) {
    let now = Arc::new(Mutex::new(0.0_f64));
    let rows = Arc::new(ScriptedEndUsers::new(answer));
    let identity = EndUserIdResolver {
        validate_end_user_id_in_db: true,
        max_end_user_budget_id: None,
        lookups: Some(DatabaseLookups {
            end_users: Arc::<ScriptedEndUsers>::clone(&rows),
            users: Arc::new(ScriptedUsers::new(Answer::Missing)),
            deadline_seconds: 1.0_f64,
        }),
        cache: Arc::new(timed_cache(&now)),
    };
    let first = identity.resolve(&request("alice")).await.unwrap();
    *now.lock().unwrap() = before;
    assert_eq!(identity.resolve(&request("alice")).await.unwrap(), first);
    assert_eq!(rows.calls().len(), 1);
    *now.lock().unwrap() = after;
    assert_eq!(identity.resolve(&request("alice")).await.unwrap(), first);
    assert_eq!(rows.calls().len(), 2);
}

#[tokio::test]
async fn validation_cache_round_trips() {
    let storage: Arc<dyn ValidationCache> = Arc::new(test_cache());
    let cache_key = "end_user_validation:alice";
    assert_eq!(storage.read(cache_key).await.expect("the empty read"), None);
    storage
        .write(
            cache_key,
            &Json::Text("valid".to_owned()),
            WriteTtl {
                seconds: POSITIVE_TTL_SECONDS,
            },
        )
        .await
        .expect("the cache write");
    assert_eq!(
        storage.read(cache_key).await.expect("the cached read"),
        Some(Json::Text("valid".to_owned()))
    );
}

#[tokio::test]
async fn end_user_adapter_reads_the_row_table() {
    let rows = Arc::new(ScriptedRows::new(RowAnswer::Missing, RowAnswer::Missing));
    rows.seed(end_user_row("alice"));
    let existence = adapter(&rows, Arc::new(RowCache::new()), None);
    assert!(
        existence
            .end_user_exists("alice")
            .await
            .expect("the row read")
    );
    assert!(
        !existence
            .end_user_exists("ghost")
            .await
            .expect("the row read")
    );
    assert_eq!(rows.queries(), vec!["alice".to_owned(), "ghost".to_owned()]);
}

#[tokio::test]
async fn end_user_adapter_without_rows_is_no_database() {
    let existence = EndUserObjectExistence::<ScriptedRows, ScriptedRows, RowCache> {
        rows: None,
        budgets: None,
        cache: Arc::new(RowCache::new()),
        registry: Arc::new(registry(true, None)),
        store: PostgresStore { pool: None },
        max_end_user_budget_id: None,
    };
    let error = existence
        .end_user_exists("alice")
        .await
        .expect_err("the missing database");
    assert!(matches!(
        error,
        ExistenceError::EndUser(EndUserObjectError::NoDatabase)
    ));
}

#[tokio::test]
async fn end_user_adapter_serves_a_cached_row() {
    let rows = Arc::new(ScriptedRows::new(RowAnswer::Missing, RowAnswer::Missing));
    let cache = Arc::new(RowCache::new());
    cache.seed_row("end_user_id:alice", &end_user_row("alice"));
    let existence = adapter(&rows, Arc::clone(&cache), None);
    assert!(
        existence
            .end_user_exists("alice")
            .await
            .expect("the cached row")
    );
    assert!(rows.queries().is_empty());
    assert_eq!(cache.reads(), vec!["end_user_id:alice".to_owned()]);
}

#[tokio::test]
async fn end_user_adapter_reports_a_cache_failure() {
    let rows = Arc::new(ScriptedRows::new(RowAnswer::Missing, RowAnswer::Missing));
    let cache = Arc::new(RowCache::new());
    cache.seed_json("end_user_id:alice", Json::Text("not a row".to_owned()));
    let existence = adapter(&rows, cache, None);
    let error = existence
        .end_user_exists("alice")
        .await
        .expect_err("the broken cached row");
    assert!(matches!(
        error,
        ExistenceError::EndUser(EndUserObjectError::CacheRead(_))
    ));
    assert!(rows.queries().is_empty());
}

// B739's caught database block answers a failed row read as no row, so the user probe still decides
#[tokio::test]
async fn end_user_adapter_swallows_a_row_failure() {
    let rows = Arc::new(ScriptedRows::new(RowAnswer::Failure, RowAnswer::Missing));
    let existence = adapter(&rows, Arc::new(RowCache::new()), None);
    assert!(
        !existence
            .end_user_exists("alice")
            .await
            .expect("the swallowed row failure")
    );
    assert_eq!(rows.queries(), vec!["alice".to_owned()]);
    let users = Arc::new(ScriptedUsers::new(Answer::Found));
    let identity = EndUserIdResolver {
        validate_end_user_id_in_db: true,
        max_end_user_budget_id: None,
        lookups: Some(DatabaseLookups {
            end_users: Arc::new(existence),
            users: Arc::<ScriptedUsers>::clone(&users),
            deadline_seconds: 1.0_f64,
        }),
        cache: Arc::new(ScriptedCache::new()),
    };
    let resolved = identity
        .resolve(&request("alice"))
        .await
        .expect("the lookup");
    assert_eq!(resolved.as_deref(), Some("alice"));
    assert_eq!(users.probes().len(), 1);
}

#[tokio::test]
async fn budget_failure_falls_through_to_the_user() {
    let (log, _guard) = capture_events();
    let rows = Arc::new(ScriptedRows::new(RowAnswer::Missing, RowAnswer::Missing));
    let cache = Arc::new(RowCache::new());
    cache.seed_row("end_user_id:alice", &end_user_row("alice"));
    cache.seed_json(
        "default_end_user_budget:proxy",
        Json::Text("broken".to_owned()),
    );
    let existence = adapter(&rows, Arc::clone(&cache), Some("proxy"));
    let error = existence
        .end_user_exists("alice")
        .await
        .expect_err("the budget failure");
    assert!(matches!(
        error,
        ExistenceError::EndUser(EndUserObjectError::Defaults(_))
    ));
    let identity = EndUserIdResolver {
        validate_end_user_id_in_db: true,
        max_end_user_budget_id: Some("proxy".to_owned()),
        lookups: Some(DatabaseLookups {
            end_users: Arc::new(existence),
            users: Arc::new(ScriptedUsers::new(Answer::Found)),
            deadline_seconds: 1.0_f64,
        }),
        cache: Arc::new(ScriptedCache::new()),
    };
    let resolved = identity
        .resolve(&request("alice"))
        .await
        .expect("the lookup");
    assert_eq!(resolved.as_deref(), Some("alice"));
    let messages = debug_messages(&log);
    assert_eq!(messages.len(), 1);
    assert!(messages[0].starts_with("end_user validation: get_end_user_object lookup failed:"));
}

#[tokio::test]
async fn real_user_lookup_probes_by_email() {
    let store = Arc::new(ScriptedUserStore::default());
    store.answer_email("alice", "alice@example.com");
    let cache = Arc::new(test_cache());
    let lookup = user_lookup(&cache, &store);
    let by_email = lookup
        .user_exists(&UserProbe {
            user_id: "alice@example.com",
            user_email: Some("alice@example.com"),
            user_id_upsert: false,
            check_db_only: false,
        })
        .await
        .expect("the email probe");
    assert!(by_email);
    let by_id = lookup
        .user_exists(&UserProbe {
            user_id: "bob",
            user_email: None,
            user_id_upsert: false,
            check_db_only: false,
        })
        .await
        .expect("the id probe");
    assert!(!by_id);
    assert_eq!(
        store.calls(),
        vec!["find_by_id", "find_by_email", "find_by_id"]
    );
    assert!(!store.calls().contains(&"create"));
    assert!(
        cache
            .read("alice@example.com")
            .await
            .expect("the email row reads back")
            .is_some(),
        "the email probe caches the row under the requested id"
    );
    lookup.tasks.close();
    lookup.tasks.wait().await;
}

#[tokio::test]
async fn real_user_lookup_never_creates_a_user() {
    let store = Arc::new(ScriptedUserStore::default());
    let cache = Arc::new(test_cache());
    let lookup = Arc::new(user_lookup(&cache, &store));
    let users: Arc<dyn UserExistence> = Arc::<UserLookup>::clone(&lookup);
    let end_users = Arc::new(ScriptedEndUsers::new(Answer::Missing));
    let identity = EndUserIdResolver {
        validate_end_user_id_in_db: true,
        max_end_user_budget_id: None,
        lookups: Some(DatabaseLookups {
            end_users: Arc::<ScriptedEndUsers>::clone(&end_users),
            users,
            deadline_seconds: 1.0_f64,
        }),
        cache: Arc::new(ScriptedCache::new()),
    };
    let resolved = identity
        .resolve(&request("ghost"))
        .await
        .expect("the lookup");
    assert!(resolved.is_none());
    assert!(!store.calls().contains(&"create"));
    assert!(
        cache
            .read("ghost")
            .await
            .expect("the user cache reads back")
            .is_none(),
        "the probe caches no user row"
    );
    lookup.tasks.close();
    lookup.tasks.wait().await;
}
