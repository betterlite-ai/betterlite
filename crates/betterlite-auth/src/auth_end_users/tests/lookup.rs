#![cfg(test)]

use betterlite_config::sections::end_user_registry::Settings;
use betterlite_db::postgres_store::session::PostgresStore;
use rust_decimal::Decimal;

use super::super::object::{EndUserObjectError, EndUserObjects, EndUserRequest};
use super::support::{
    BudgetAnswer, EndUserAnswer, ScriptedBudgetRows, ScriptedCache, ScriptedEndUserRows, Surface,
    budget_fixture, end_user_row, money,
};
use crate::end_user_registry::EndUserRegistry;

fn guard_off() -> EndUserRegistry {
    EndUserRegistry::new(Settings {
        validate_end_user_id_in_db: true,
        max_end_user_budget_id: None,
    })
}

fn registry_on() -> EndUserRegistry {
    EndUserRegistry::new(Settings {
        validate_end_user_id_in_db: false,
        max_end_user_budget_id: None,
    })
}

fn request<'req>(
    end_user_id: Option<&'req str>,
    key_end_user_budget_id: Option<&'req str>,
    token_end_user_max_budget: Option<Decimal>,
) -> EndUserRequest<'req> {
    EndUserRequest {
        end_user_id,
        key_end_user_budget_id,
        token_end_user_max_budget,
    }
}

#[tokio::test]
async fn own_resolved_budget_wins() {
    let own = budget_fixture("own", Some("100"));
    let row = end_user_row("alice", "0", Some("own"), Some(&own));
    let rows = ScriptedEndUserRows::answering(EndUserAnswer::Row(Some(row.clone())));
    let budgets = ScriptedBudgetRows::answering(BudgetAnswer::QueryFailure);
    let cache = ScriptedCache::new();
    let registry = guard_off();
    let store = PostgresStore { pool: None };
    let objects = EndUserObjects {
        rows: Some(&rows),
        budgets: Some(&budgets),
        cache: &cache,
        registry: &registry,
        store: &store,
        max_end_user_budget_id: Some("proxy"),
    };
    let found = objects
        .get_end_user_object(&request(Some("alice"), Some("key"), None))
        .await
        .expect("the row resolves")
        .expect("the row exists");
    assert_eq!(found, row);
    assert!(budgets.queries().is_empty());
    assert_eq!(cache.reads(), vec!["end_user:end_user_id:alice".to_owned()]);
    assert_eq!(
        cache.writes(),
        vec!["end_user:end_user_id:alice:45".to_owned()]
    );
}

#[tokio::test]
async fn dangling_budget_uses_default() {
    let proxy = budget_fixture("proxy", Some("100"));
    let row = end_user_row("alice", "0", Some("dangling"), None);
    let rows = ScriptedEndUserRows::answering(EndUserAnswer::Row(Some(row)));
    let budgets = ScriptedBudgetRows::answering(BudgetAnswer::Row(Some(proxy.clone())));
    let cache = ScriptedCache::new();
    let registry = guard_off();
    let store = PostgresStore { pool: None };
    let objects = EndUserObjects {
        rows: Some(&rows),
        budgets: Some(&budgets),
        cache: &cache,
        registry: &registry,
        store: &store,
        max_end_user_budget_id: Some("proxy"),
    };
    let found = objects
        .get_end_user_object(&request(Some("alice"), None, None))
        .await
        .expect("the row resolves")
        .expect("the row exists");
    assert_eq!(found.budget_id.as_deref(), Some("dangling"));
    assert_eq!(found.litellm_budget_table, Some(proxy.clone()));
    let stored = cache
        .stored_end_user("end_user_id:alice")
        .expect("the fetched row is cached");
    assert_eq!(stored.litellm_budget_table, Some(proxy));
}

#[tokio::test]
async fn default_copy_preserves_id() {
    let key = budget_fixture("key-budget", Some("50"));
    let row = end_user_row("alice", "0", Some("own"), None);
    let rows = ScriptedEndUserRows::answering(EndUserAnswer::Row(Some(row.clone())));
    let budgets = ScriptedBudgetRows::answering(BudgetAnswer::Row(None));
    budgets.set("key", BudgetAnswer::Row(Some(key.clone())));
    let cache = ScriptedCache::new();
    let registry = guard_off();
    let store = PostgresStore { pool: None };
    let objects = EndUserObjects {
        rows: Some(&rows),
        budgets: Some(&budgets),
        cache: &cache,
        registry: &registry,
        store: &store,
        max_end_user_budget_id: None,
    };
    let found = objects
        .get_end_user_object(&request(Some("alice"), Some("key"), None))
        .await
        .expect("the row resolves")
        .expect("the row exists");
    assert_eq!(found.budget_id.as_deref(), Some("own"));
    assert_eq!(found.litellm_budget_table, Some(key));
    let stored = cache
        .stored_end_user("end_user_id:alice")
        .expect("the fetched row is cached");
    assert_eq!(stored.budget_id.as_deref(), Some("own"));
    assert!(stored.litellm_budget_table.is_none());
}

#[tokio::test]
async fn db_required_before_none_id() {
    let cache = ScriptedCache::new();
    let registry = guard_off();
    let store = PostgresStore { pool: None };
    let objects: EndUserObjects<'_, ScriptedEndUserRows, ScriptedBudgetRows, ScriptedCache> =
        EndUserObjects {
            rows: None,
            budgets: None,
            cache: &cache,
            registry: &registry,
            store: &store,
            max_end_user_budget_id: None,
        };
    let error = objects
        .get_end_user_object(&request(None, None, None))
        .await
        .expect_err("the absent database fails before the absent id");
    assert!(matches!(error, EndUserObjectError::NoDatabase));
    assert!(cache.reads().is_empty());
}

#[tokio::test]
async fn empty_id_queries() {
    let row = end_user_row("", "0", None, None);
    let rows = ScriptedEndUserRows::answering(EndUserAnswer::Row(Some(row)));
    let budgets = ScriptedBudgetRows::answering(BudgetAnswer::Row(None));
    let cache = ScriptedCache::new();
    let registry = guard_off();
    let store = PostgresStore { pool: None };
    let objects = EndUserObjects {
        rows: Some(&rows),
        budgets: Some(&budgets),
        cache: &cache,
        registry: &registry,
        store: &store,
        max_end_user_budget_id: None,
    };
    assert!(
        objects
            .get_end_user_object(&request(Some(""), None, None))
            .await
            .expect("the empty id is a real lookup")
            .is_some()
    );
    assert_eq!(rows.queries(), vec![String::new()]);
}

#[tokio::test]
async fn unrestricted_registry_skips_row() {
    let rows = ScriptedEndUserRows::answering(EndUserAnswer::Failure);
    let budgets = ScriptedBudgetRows::answering(BudgetAnswer::Row(None));
    let cache = ScriptedCache::new();
    cache.put_registry(&["other-user"]);
    let registry = registry_on();
    let store = PostgresStore { pool: None };
    let objects = EndUserObjects {
        rows: Some(&rows),
        budgets: Some(&budgets),
        cache: &cache,
        registry: &registry,
        store: &store,
        max_end_user_budget_id: None,
    };
    assert!(
        objects
            .get_end_user_object(&request(Some("alice"), None, None))
            .await
            .expect("the registry answers")
            .is_none()
    );
    assert!(rows.queries().is_empty());
    assert_eq!(
        cache.reads(),
        vec![
            "end_user:end_user_id:alice".to_owned(),
            "registry:end_user_restricted_registry".to_owned(),
        ]
    );
}

#[tokio::test]
async fn budget_forces_registry_lookup() {
    let row = end_user_row("alice", "0", None, None);
    let rows = ScriptedEndUserRows::answering(EndUserAnswer::Row(Some(row)));
    let budgets = ScriptedBudgetRows::answering(BudgetAnswer::Row(None));
    let cache = ScriptedCache::new();
    cache.put_registry(&["other-user"]);
    let registry = registry_on();
    let store = PostgresStore { pool: None };
    let objects = EndUserObjects {
        rows: Some(&rows),
        budgets: Some(&budgets),
        cache: &cache,
        registry: &registry,
        store: &store,
        max_end_user_budget_id: None,
    };
    assert!(
        objects
            .get_end_user_object(&request(Some("alice"), Some("key"), None))
            .await
            .expect("the key budget forces the row read")
            .is_some()
    );
    assert_eq!(rows.queries(), vec!["alice".to_owned()]);
    assert!(
        !cache
            .reads()
            .iter()
            .any(|entry| entry.starts_with("registry:"))
    );

    assert!(
        objects
            .get_end_user_object(&request(Some("bob"), None, Some(money("5"))))
            .await
            .expect("the token budget forces the row read")
            .is_some()
    );
    assert_eq!(rows.queries(), vec!["alice".to_owned(), "bob".to_owned()]);
}

#[tokio::test]
async fn proxy_default_cached_before_key_default() {
    let proxy = budget_fixture("proxy", Some("100"));
    let key = budget_fixture("key-budget", Some("50"));
    let row = end_user_row("alice", "0", None, None);
    let rows = ScriptedEndUserRows::answering(EndUserAnswer::Row(Some(row)));
    let budgets = ScriptedBudgetRows::answering(BudgetAnswer::Row(None));
    budgets.set("proxy", BudgetAnswer::Row(Some(proxy)));
    budgets.set("key", BudgetAnswer::Row(Some(key.clone())));
    let cache = ScriptedCache::new();
    let registry = guard_off();
    let store = PostgresStore { pool: None };
    let objects = EndUserObjects {
        rows: Some(&rows),
        budgets: Some(&budgets),
        cache: &cache,
        registry: &registry,
        store: &store,
        max_end_user_budget_id: Some("proxy"),
    };
    let found = objects
        .get_end_user_object(&request(Some("alice"), Some("key"), None))
        .await
        .expect("the row resolves")
        .expect("the row exists");
    assert_eq!(found.litellm_budget_table, Some(key));
    assert_eq!(
        cache.writes(),
        vec![
            "budget:default_end_user_budget:proxy:45".to_owned(),
            "end_user:end_user_id:alice:45".to_owned(),
            "budget:default_end_user_budget:key:45".to_owned(),
        ]
    );
    assert_eq!(
        cache.reads(),
        vec![
            "end_user:end_user_id:alice".to_owned(),
            "budget:default_end_user_budget:proxy".to_owned(),
            "budget:default_end_user_budget:key".to_owned(),
        ]
    );
}

#[tokio::test]
async fn two_keys_do_not_share_defaults() {
    let proxy = budget_fixture("proxy", Some("100"));
    let key_a = budget_fixture("key-a", Some("50"));
    let key_b = budget_fixture("key-b", Some("25"));
    let row = end_user_row("alice", "0", None, None);
    let rows = ScriptedEndUserRows::answering(EndUserAnswer::Row(Some(row)));
    let budgets = ScriptedBudgetRows::answering(BudgetAnswer::Row(None));
    budgets.set("proxy", BudgetAnswer::Row(Some(proxy.clone())));
    budgets.set("key-a", BudgetAnswer::Row(Some(key_a.clone())));
    budgets.set("key-b", BudgetAnswer::Row(Some(key_b.clone())));
    let cache = ScriptedCache::new();
    let registry = guard_off();
    let store = PostgresStore { pool: None };
    let objects = EndUserObjects {
        rows: Some(&rows),
        budgets: Some(&budgets),
        cache: &cache,
        registry: &registry,
        store: &store,
        max_end_user_budget_id: Some("proxy"),
    };
    let first = objects
        .get_end_user_object(&request(Some("alice"), Some("key-a"), None))
        .await
        .expect("the first key resolves")
        .expect("the row exists");
    assert_eq!(first.litellm_budget_table, Some(key_a));
    let second = objects
        .get_end_user_object(&request(Some("alice"), Some("key-b"), None))
        .await
        .expect("the second key resolves from the cache")
        .expect("the row exists");
    assert_eq!(second.litellm_budget_table, Some(key_b));
    let stored = cache
        .stored_end_user("end_user_id:alice")
        .expect("the fetched row is cached");
    assert_eq!(stored.litellm_budget_table, Some(proxy));
    assert_eq!(
        cache.writes(),
        vec![
            "budget:default_end_user_budget:proxy:45".to_owned(),
            "end_user:end_user_id:alice:45".to_owned(),
            "budget:default_end_user_budget:key-a:45".to_owned(),
            "budget:default_end_user_budget:key-b:45".to_owned(),
        ]
    );
}

#[tokio::test]
async fn cached_default_resolved_again() {
    let proxy = budget_fixture("proxy", Some("100"));
    let key = budget_fixture("key-budget", Some("25"));
    let cached = end_user_row("alice", "0", None, Some(&proxy));
    let rows = ScriptedEndUserRows::answering(EndUserAnswer::Failure);
    let budgets = ScriptedBudgetRows::answering(BudgetAnswer::Row(None));
    budgets.set("key", BudgetAnswer::Row(Some(key.clone())));
    let cache = ScriptedCache::new();
    cache.put_end_user("end_user_id:alice", cached);
    let registry = guard_off();
    let store = PostgresStore { pool: None };
    let objects = EndUserObjects {
        rows: Some(&rows),
        budgets: Some(&budgets),
        cache: &cache,
        registry: &registry,
        store: &store,
        max_end_user_budget_id: None,
    };
    let found = objects
        .get_end_user_object(&request(Some("alice"), Some("key"), None))
        .await
        .expect("the cached row resolves")
        .expect("the row exists");
    assert_eq!(found.litellm_budget_table, Some(key));
    assert!(rows.queries().is_empty());
    assert!(cache.writes().is_empty());
    assert_eq!(
        cache.reads(),
        vec![
            "end_user:end_user_id:alice".to_owned(),
            "budget:default_end_user_budget:key".to_owned(),
        ]
    );
}

#[tokio::test]
async fn db_block_failure_returns_none() {
    let failing = ScriptedEndUserRows::answering(EndUserAnswer::Failure);
    let budgets = ScriptedBudgetRows::answering(BudgetAnswer::Row(None));
    let cache = ScriptedCache::new();
    let registry = guard_off();
    let store = PostgresStore { pool: None };
    let objects = EndUserObjects {
        rows: Some(&failing),
        budgets: Some(&budgets),
        cache: &cache,
        registry: &registry,
        store: &store,
        max_end_user_budget_id: None,
    };
    assert!(
        objects
            .get_end_user_object(&request(Some("alice"), Some("key"), None))
            .await
            .expect("a failed row read is silent")
            .is_none()
    );
    assert!(cache.writes().is_empty());

    let missing = ScriptedEndUserRows::answering(EndUserAnswer::Row(None));
    let objects = EndUserObjects {
        rows: Some(&missing),
        budgets: Some(&budgets),
        cache: &cache,
        registry: &registry,
        store: &store,
        max_end_user_budget_id: None,
    };
    assert!(
        objects
            .get_end_user_object(&request(Some("alice"), None, None))
            .await
            .expect("a missing row is silent")
            .is_none()
    );
    assert!(cache.writes().is_empty());
}

#[tokio::test]
async fn prequery_failure_propagates() {
    let rows = ScriptedEndUserRows::answering(EndUserAnswer::Row(None));
    let budgets = ScriptedBudgetRows::answering(BudgetAnswer::Row(None));
    let cache = ScriptedCache::failing(Some(Surface::EndUser), None);
    let registry = guard_off();
    let store = PostgresStore { pool: None };
    let objects = EndUserObjects {
        rows: Some(&rows),
        budgets: Some(&budgets),
        cache: &cache,
        registry: &registry,
        store: &store,
        max_end_user_budget_id: None,
    };
    let error = objects
        .get_end_user_object(&request(Some("alice"), None, None))
        .await
        .expect_err("the cache read sits outside the catch");
    assert!(matches!(error, EndUserObjectError::CacheRead(_)));
    assert!(rows.queries().is_empty());

    let cache = ScriptedCache::failing(Some(Surface::Registry), None);
    let registry = registry_on();
    let objects = EndUserObjects {
        rows: Some(&rows),
        budgets: Some(&budgets),
        cache: &cache,
        registry: &registry,
        store: &store,
        max_end_user_budget_id: None,
    };
    let error = objects
        .get_end_user_object(&request(Some("alice"), None, None))
        .await
        .expect_err("the registry read sits outside the catch");
    assert!(matches!(error, EndUserObjectError::Registry(_)));
}
