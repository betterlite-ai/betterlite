#![cfg(test)]

use std::sync::Arc;

use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_models::decisions::Json;

use super::super::cache::{POSITIVE_TTL_SECONDS, ValidationCache};
use super::super::existence::{
    EndUserExistence as _, EndUserObjectExistence, ExistenceError, UserExistence, UserProbe,
};
use super::super::resolve::{DatabaseLookups, EndUserIdResolver};
use super::support::{
    Answer, RowAnswer, RowCache, ScriptedCache, ScriptedEndUsers, ScriptedRows, ScriptedUserStore,
    ScriptedUsers, UserCachePort, capture_events, debug_messages, end_user_row, registry, request,
    test_cache, user_lookup,
};
use crate::auth_cache::WriteTtl;
use crate::auth_end_users::object::EndUserObjectError;
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
    let user_cache = Arc::new(UserCachePort::default());
    let lookup = user_lookup(&user_cache, &store);
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
    assert_eq!(
        user_cache.write_keys(),
        vec!["alice@example.com".to_owned()]
    );
    lookup.tasks.close();
    lookup.tasks.wait().await;
}

#[tokio::test]
async fn real_user_lookup_never_creates_a_user() {
    let store = Arc::new(ScriptedUserStore::default());
    let user_cache = Arc::new(UserCachePort::default());
    let lookup = Arc::new(user_lookup(&user_cache, &store));
    let users: Arc<dyn UserExistence> = Arc::<UserLookup>::clone(&lookup);
    let end_users = Arc::new(ScriptedEndUsers::new(Answer::Missing));
    let identity = EndUserIdResolver {
        validate_end_user_id_in_db: true,
        max_end_user_budget_id: None,
        lookups: Some(DatabaseLookups {
            end_users: Arc::<ScriptedEndUsers>::clone(&end_users),
            users,
        }),
        cache: Arc::new(ScriptedCache::new()),
    };
    let resolved = identity
        .resolve(&request("ghost"))
        .await
        .expect("the lookup");
    assert!(resolved.is_none());
    assert!(!store.calls().contains(&"create"));
    assert!(user_cache.write_keys().is_empty());
    lookup.tasks.close();
    lookup.tasks.wait().await;
}
