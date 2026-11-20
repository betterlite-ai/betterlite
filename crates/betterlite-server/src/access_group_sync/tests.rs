#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_auth::auth_invalidation::error::InvalidationError;
use rust_decimal::Decimal;
use tokio::sync::Barrier;
use tokio::time::timeout;

use super::cache::invalidate_access_group_caches;
use super::fixtures::{
    KeyCall, ScriptedCache, ScriptedKeyDatabase, ScriptedTeamTransaction, TeamStep,
};
use super::key::{
    KeyAccessGroupUpdate, KeyMembershipStatement, KeyRegeneration, KeySyncError,
    sync_key_access_groups, sync_key_regeneration, sync_key_update_access_groups,
};
use super::team::{TEAM_ADVISORY_LOCK_SQL, reconcile_team_access_groups};

mod postgres;

fn groups(entries: &[(&str, &[&str])]) -> BTreeMap<String, Vec<String>> {
    entries
        .iter()
        .map(|(access_group_id, tokens)| {
            let tokens = tokens.iter().map(|token| (*token).to_owned()).collect();
            ((*access_group_id).to_owned(), tokens)
        })
        .collect()
}

fn team_groups(entries: &[(&str, Option<&[&str]>)]) -> BTreeMap<String, Option<Vec<String>>> {
    entries
        .iter()
        .map(|(access_group_id, teams)| {
            let teams = teams.map(|names| names.iter().map(|team| (*team).to_owned()).collect());
            ((*access_group_id).to_owned(), teams)
        })
        .collect()
}

fn moved_call(
    statement: KeyMembershipStatement,
    key_token: &str,
    access_group_ids: &[&str],
) -> KeyCall {
    KeyCall::Move {
        statement,
        key_token: key_token.to_owned(),
        access_group_ids: access_group_ids
            .iter()
            .map(|access_group_id| (*access_group_id).to_owned())
            .collect(),
    }
}

fn repoint_call(previous: &str, new: &str) -> KeyCall {
    KeyCall::Repoint {
        previous_key_token: previous.to_owned(),
        new_key_token: new.to_owned(),
    }
}

const fn regeneration<'request>(
    previous: &'request str,
    new: &'request str,
    update: Option<KeyAccessGroupUpdate<'request>>,
    existing: Option<&'request [String]>,
) -> KeyRegeneration<'request> {
    KeyRegeneration {
        previous_key_token: previous,
        new_key_token: new,
        update,
        existing_access_group_ids: existing,
    }
}

#[tokio::test]
async fn key_set_delta_order() {
    let mut database = ScriptedKeyDatabase::new(groups(&[
        ("a", &["tok"]),
        ("b", &["tok"]),
        ("c", &[]),
        ("d", &[]),
    ]));
    database.reverse_rows = true;
    let cache = ScriptedCache::new();
    let previous = vec!["b".to_owned(), "a".to_owned()];
    let updated = vec!["d".to_owned(), "c".to_owned()];
    sync_key_access_groups(&database, &cache, "tok", Some(&previous), Some(&updated))
        .await
        .unwrap();
    assert_eq!(
        *database.calls.lock().expect("calls"),
        vec![
            moved_call(KeyMembershipStatement::Attach, "tok", &["c", "d"]),
            moved_call(KeyMembershipStatement::Detach, "tok", &["a", "b"]),
        ]
    );
    assert_eq!(
        *cache.calls.lock().expect("calls"),
        vec![
            "d".to_owned(),
            "c".to_owned(),
            "b".to_owned(),
            "a".to_owned(),
        ]
    );
    let groups = database.groups.lock().expect("groups");
    assert_eq!(groups.get("a"), Some(&Vec::new()));
    assert_eq!(groups.get("c"), Some(&vec!["tok".to_owned()]));
}

#[tokio::test]
async fn unchanged_groups_no_io() {
    let database = ScriptedKeyDatabase::new(groups(&[("a", &["tok"])]));
    let cache = ScriptedCache::new();
    let same = vec!["a".to_owned()];
    sync_key_access_groups(&database, &cache, "tok", Some(&same), Some(&same))
        .await
        .unwrap();
    sync_key_access_groups(&database, &cache, "tok", None, None)
        .await
        .unwrap();
    assert!(database.calls.lock().expect("calls").is_empty());
    assert!(cache.calls.lock().expect("calls").is_empty());
}

#[tokio::test]
async fn attach_cache_failure_stops_detach() {
    let database = ScriptedKeyDatabase::new(groups(&[("a", &["tok"]), ("b", &[])]));
    let mut cache = ScriptedCache::new();
    cache.failures = Mutex::new(BTreeMap::from([("b".to_owned(), 7)]));
    let previous = vec!["a".to_owned()];
    let updated = vec!["b".to_owned()];
    let error = sync_key_access_groups(&database, &cache, "tok", Some(&previous), Some(&updated))
        .await
        .expect_err("the attach eviction fails");
    assert!(
        matches!(error, KeySyncError::Cache(InvalidationError::SpendOutOfRange(value)) if value == Decimal::new(7, 0))
    );
    assert_eq!(*cache.calls.lock().expect("calls"), vec!["b".to_owned()]);
    let calls = database.calls.lock().expect("calls");
    assert_eq!(calls.len(), 1);
    assert!(matches!(
        calls.first(),
        Some(KeyCall::Move {
            statement: KeyMembershipStatement::Attach,
            ..
        })
    ));
    let groups = database.groups.lock().expect("groups");
    assert_eq!(groups.get("b"), Some(&vec!["tok".to_owned()]));
    assert_eq!(groups.get("a"), Some(&vec!["tok".to_owned()]));
}

#[tokio::test]
async fn key_retry_only_moved_caches() {
    let database = ScriptedKeyDatabase::new(groups(&[("a", &["tok"]), ("b", &[])]));
    let mut cache = ScriptedCache::new();
    cache.failures = Mutex::new(BTreeMap::from([("b".to_owned(), 7)]));
    let previous = vec!["a".to_owned()];
    let updated = vec!["b".to_owned()];
    let error = sync_key_access_groups(&database, &cache, "tok", Some(&previous), Some(&updated))
        .await
        .expect_err("the first attempt fails");
    assert!(matches!(error, KeySyncError::Cache(_)));
    let retry = ScriptedCache::new();
    sync_key_access_groups(&database, &retry, "tok", Some(&previous), Some(&updated))
        .await
        .unwrap();
    assert_eq!(*retry.calls.lock().expect("calls"), vec!["a".to_owned()]);
    let calls = database.calls.lock().expect("calls");
    assert_eq!(calls.len(), 3);
    assert_eq!(
        calls.get(2),
        Some(&moved_call(KeyMembershipStatement::Detach, "tok", &["a"]))
    );
}

#[tokio::test]
async fn update_absent_vs_null() {
    let database = ScriptedKeyDatabase::new(groups(&[("a", &["tok"]), ("b", &["tok"])]));
    let cache = ScriptedCache::new();
    let first = vec!["a".to_owned()];
    sync_key_update_access_groups(
        &database,
        &cache,
        "tok",
        KeyAccessGroupUpdate::Absent,
        Some(&first),
    )
    .await
    .unwrap();
    assert!(database.calls.lock().expect("calls").is_empty());
    assert!(cache.calls.lock().expect("calls").is_empty());
    sync_key_update_access_groups(
        &database,
        &cache,
        "tok",
        KeyAccessGroupUpdate::Cleared,
        Some(&first),
    )
    .await
    .unwrap();
    assert_eq!(
        *database.calls.lock().expect("calls"),
        vec![moved_call(KeyMembershipStatement::Detach, "tok", &["a"])]
    );
    assert_eq!(*cache.calls.lock().expect("calls"), vec!["a".to_owned()]);
    assert_eq!(
        database.groups.lock().expect("groups").get("a"),
        Some(&Vec::new())
    );
}

#[tokio::test]
async fn update_empty_list_clears() {
    let database = ScriptedKeyDatabase::new(groups(&[("a", &["tok"]), ("b", &["tok"])]));
    let cache = ScriptedCache::new();
    let second = vec!["b".to_owned()];
    let empty: [String; 0] = [];
    sync_key_update_access_groups(
        &database,
        &cache,
        "tok",
        KeyAccessGroupUpdate::Groups(&empty),
        Some(&second),
    )
    .await
    .unwrap();
    assert_eq!(
        *database.calls.lock().expect("calls"),
        vec![moved_call(KeyMembershipStatement::Detach, "tok", &["b"])]
    );
    assert_eq!(*cache.calls.lock().expect("calls"), vec!["b".to_owned()]);
    assert_eq!(
        database.groups.lock().expect("groups").get("b"),
        Some(&Vec::new())
    );
}

#[tokio::test]
async fn regeneration_live_groups() {
    let database = ScriptedKeyDatabase::new(groups(&[
        ("g1", &["old", "old"]),
        ("g2", &["new", "old"]),
        ("g3", &["other"]),
    ]));
    let cache = ScriptedCache::new();
    let updated = vec!["g3".to_owned()];
    let existing = vec!["g2".to_owned()];
    let update = KeyAccessGroupUpdate::Groups(&updated);
    sync_key_regeneration(
        &database,
        &cache,
        regeneration("old", "new", Some(update), Some(&existing)),
    )
    .await
    .unwrap();
    sync_key_regeneration(&database, &cache, regeneration("new", "new", None, None))
        .await
        .unwrap();
    assert_eq!(
        *cache.calls.lock().expect("calls"),
        ["g1", "g2", "g3", "g2", "g1", "g3"]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<String>>()
    );
    let groups = database.groups.lock().expect("groups");
    assert_eq!(groups.get("g1"), Some(&vec!["new".to_owned()]));
    assert_eq!(groups.get("g2"), Some(&Vec::new()));
    assert_eq!(groups.get("g3"), Some(&vec!["new".to_owned()]));
}

#[tokio::test]
async fn regeneration_failure_order() {
    let mut database = ScriptedKeyDatabase::new(groups(&[("a", &["old"]), ("b", &[])]));
    database.fail_statement = Some(KeyMembershipStatement::Attach);
    let cache = ScriptedCache::new();
    let updated = vec!["b".to_owned()];
    let update = KeyAccessGroupUpdate::Groups(&updated);
    let error = sync_key_regeneration(
        &database,
        &cache,
        regeneration("old", "new", Some(update), None),
    )
    .await
    .expect_err("the update attach fails");
    assert!(matches!(error, KeySyncError::Database(_)));
    assert_eq!(
        database.groups.lock().expect("groups").get("a"),
        Some(&vec!["new".to_owned()])
    );
    assert_eq!(*cache.calls.lock().expect("calls"), vec!["a".to_owned()]);
    let calls = database.calls.lock().expect("calls");
    assert_eq!(calls.len(), 2);
    assert!(matches!(calls.first(), Some(KeyCall::Repoint { .. })));
    assert!(matches!(
        calls.get(1),
        Some(KeyCall::Move {
            statement: KeyMembershipStatement::Attach,
            ..
        })
    ));
}

#[tokio::test]
async fn team_lock_read_reconcile_order() {
    let transaction = ScriptedTeamTransaction::new(
        Some(vec!["g1".to_owned(), "g3".to_owned()]),
        team_groups(&[("g1", None), ("g2", Some(&["t1"])), ("g3", Some(&["t1"]))]),
    );
    let affected = reconcile_team_access_groups(&transaction, "t1")
        .await
        .unwrap();
    assert_eq!(
        affected,
        vec!["g1".to_owned(), "g2".to_owned(), "g3".to_owned()]
    );
    assert_eq!(
        *transaction.calls.lock().expect("calls"),
        vec![
            TeamStep::Lock,
            TeamStep::Read,
            TeamStep::Affected,
            TeamStep::Attach,
            TeamStep::Detach,
        ]
    );
    let groups = transaction.groups.lock().expect("groups");
    assert_eq!(groups.get("g1"), Some(&Some(vec!["t1".to_owned()])));
    assert_eq!(groups.get("g2"), Some(&Some(Vec::new())));
    assert_eq!(
        TEAM_ADVISORY_LOCK_SQL,
        "SELECT pg_advisory_xact_lock(hashtext($1)) IS NULL AS locked"
    );
}

#[tokio::test]
async fn missing_team_detaches_all() {
    let mut transaction = ScriptedTeamTransaction::new(
        None,
        team_groups(&[("g1", Some(&["t1"])), ("g2", Some(&["t1", "other"]))]),
    );
    transaction.missing_team = true;
    let affected = reconcile_team_access_groups(&transaction, "t1")
        .await
        .unwrap();
    assert_eq!(affected, vec!["g1".to_owned(), "g2".to_owned()]);
    let groups = transaction.groups.lock().expect("groups");
    assert_eq!(groups.get("g1"), Some(&Some(Vec::new())));
    assert_eq!(groups.get("g2"), Some(&Some(vec!["other".to_owned()])));
    assert_eq!(
        *transaction.calls.lock().expect("calls"),
        vec![
            TeamStep::Lock,
            TeamStep::Read,
            TeamStep::Affected,
            TeamStep::Attach,
            TeamStep::Detach,
        ]
    );
}

#[tokio::test]
async fn team_null_arrays_and_unchanged_groups() {
    let transaction = ScriptedTeamTransaction::new(
        None,
        team_groups(&[("g1", None), ("g2", Some(&["t1", "other"]))]),
    );
    let affected = reconcile_team_access_groups(&transaction, "t1")
        .await
        .unwrap();
    assert_eq!(affected, vec!["g2".to_owned()]);
    assert_eq!(
        transaction.groups.lock().expect("groups").get("g2"),
        Some(&Some(vec!["other".to_owned()]))
    );
    let unchanged = ScriptedTeamTransaction::new(
        Some(vec!["g1".to_owned()]),
        team_groups(&[("g1", Some(&["t1"])), ("g2", None)]),
    );
    let affected = reconcile_team_access_groups(&unchanged, "t1")
        .await
        .unwrap();
    assert_eq!(affected, vec!["g1".to_owned()]);
    assert_eq!(
        unchanged.groups.lock().expect("groups").get("g1"),
        Some(&Some(vec!["t1".to_owned()]))
    );
}

#[tokio::test]
async fn all_cache_attempts_before_first_error() {
    let mut cache = ScriptedCache::new();
    cache.barrier = Some(Arc::new(Barrier::new(3)));
    cache.failures = Mutex::new(BTreeMap::from([("a".to_owned(), 1), ("b".to_owned(), 2)]));
    let ids = vec!["a".to_owned(), "b".to_owned(), "a".to_owned()];
    let outcome = timeout(
        Duration::from_secs(5),
        invalidate_access_group_caches(&cache, &ids),
    )
    .await
    .expect("every attempt runs");
    let error = outcome.expect_err("the first failure in input order");
    assert!(
        matches!(error, InvalidationError::SpendOutOfRange(value) if value == Decimal::new(1, 0))
    );
    assert_eq!(*cache.calls.lock().expect("calls"), ids);
}

#[tokio::test]
async fn postgres_pinned_rows() {
    let (fixture, pool) = postgres::database().await;
    postgres::seed_groups(&pool).await;
    postgres::seed_teams(&pool).await;
    postgres::key_pinned_rows(&pool).await;
    postgres::team_pinned_rows(&pool).await;
    postgres::close(fixture, pool).await;
}

#[tokio::test]
async fn postgres_failure_effects() {
    let (fixture, pool) = postgres::database().await;
    postgres::seed_groups(&pool).await;
    postgres::seed_teams(&pool).await;
    postgres::key_partial_write(&pool).await;
    postgres::rolled_back(&pool).await;
    postgres::commit_then_cache_failure(&pool).await;
    postgres::close(fixture, pool).await;
}

#[tokio::test]
async fn caller_transaction_rollback() {
    let (fixture, pool) = postgres::database().await;
    postgres::seed_groups(&pool).await;
    postgres::seed_teams(&pool).await;
    postgres::rolled_back(&pool).await;
    postgres::close(fixture, pool).await;
}

#[tokio::test]
async fn team_retry_invalidates_desired() {
    let (fixture, pool) = postgres::database().await;
    postgres::seed_groups(&pool).await;
    postgres::seed_teams(&pool).await;
    postgres::retry_evicts_desired(&pool).await;
    postgres::close(fixture, pool).await;
}

#[tokio::test]
async fn post_commit_eviction_failure() {
    let (fixture, pool) = postgres::database().await;
    postgres::seed_groups(&pool).await;
    postgres::seed_teams(&pool).await;
    postgres::commit_then_cache_failure(&pool).await;
    postgres::close(fixture, pool).await;
}
