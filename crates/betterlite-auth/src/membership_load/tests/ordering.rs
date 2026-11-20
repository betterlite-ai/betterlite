#![cfg(test)]

use std::sync::Arc;

use betterlite_db::database_failures::deadline::DbLookupError;
use betterlite_models::decisions::Json;
use tokio_util::task::TaskTracker;

use super::support::{
    CountingIndex, FakeCache, FakeRows, Harness, Logs, NonTaskIndex, RecordedWrite, RecordingTable,
    RowsAnswer, TableMode, WriteKind, drain, expect_absent, expect_error, expect_present, index,
    key, lookup, membership, stored_membership,
};
use crate::auth_cache::keys::NO_TEAM_MEMBERSHIP;
use crate::auth_read::table::{AuthReadError, Presence};
use crate::membership_load::error::MembershipLoadError;
use crate::membership_load::index::MembershipIndex;
use crate::membership_load::rows::{MembershipFilter, MembershipInclude, read_membership};

fn sentinel() -> Json {
    Json::Text(NO_TEAM_MEMBERSHIP.to_owned())
}

#[tokio::test]
async fn missing_id_short_circuit() {
    let cache = FakeCache::new(60.0_f64);
    let tasks = TaskTracker::new();
    let subject = lookup(Arc::clone(&cache), None, index().1, &tasks);
    expect_absent(subject.get_team_membership(None, Some("team-1")).await);
    expect_absent(subject.get_team_membership(Some("user-1"), None).await);
    assert!(cache.reads().is_empty());
}

#[tokio::test]
async fn empty_ids_are_used() {
    let cache = FakeCache::new(60.0_f64);
    let rows = FakeRows::new(RowsAnswer::Found);
    let tasks = TaskTracker::new();
    let subject = lookup(
        Arc::clone(&cache),
        Some(Arc::clone(&rows)),
        index().1,
        &tasks,
    );
    expect_present(subject.get_team_membership(Some(""), Some("")).await);
    assert_eq!(rows.last_call(), Some((String::new(), String::new())));
    assert_eq!(cache.reads(), vec![(key("", ""), true)]);
    drain(&tasks).await;
}

#[tokio::test]
async fn local_member_without_db() {
    let cache = FakeCache::new(60.0_f64);
    cache.seed_local(&key("user-1", "team-1"), stored_membership());
    let tasks = TaskTracker::new();
    let subject = lookup(Arc::clone(&cache), None, index().1, &tasks);
    let found = subject
        .get_team_membership(Some("user-1"), Some("team-1"))
        .await;
    assert_eq!(expect_present(found), membership());
    assert_eq!(cache.reads(), vec![(key("user-1", "team-1"), true)]);
    assert!(cache.writes().is_empty());
}

#[tokio::test]
async fn local_negative_without_db() {
    let cache = FakeCache::new(60.0_f64);
    cache.seed_local(&key("user-1", "team-1"), sentinel());
    let tasks = TaskTracker::new();
    let subject = lookup(Arc::clone(&cache), None, index().1, &tasks);
    let found = subject
        .get_team_membership(Some("user-1"), Some("team-1"))
        .await;
    expect_absent(found);
    assert_eq!(cache.reads(), vec![(key("user-1", "team-1"), true)]);
    assert!(cache.writes().is_empty());
}

#[tokio::test]
async fn join_before_db_requirement() {
    let cache = FakeCache::new(60.0_f64);
    let rows = FakeRows::blocked(RowsAnswer::Found);
    let shared = index().1;
    let counting = CountingIndex::new(Arc::clone(&shared));
    let index_clone = Arc::clone(&counting);
    let index_handle: Arc<dyn MembershipIndex> = index_clone;
    let tasks = TaskTracker::new();
    let creator = lookup(
        Arc::clone(&cache),
        Some(Arc::clone(&rows)),
        Arc::clone(&index_handle),
        &tasks,
    );
    let creator_task = tasks.spawn(async move {
        creator
            .get_team_membership(Some("user-1"), Some("team-1"))
            .await
    });
    rows.wait_started().await;
    counting.wait_entry().await;
    let joiner = lookup(Arc::clone(&cache), None, index_handle, &tasks);
    let joiner_task = tasks.spawn(async move {
        joiner
            .get_team_membership(Some("user-1"), Some("team-1"))
            .await
    });
    counting.wait_entry().await;
    rows.release();
    assert_eq!(
        expect_present(creator_task.await.expect("the creator")),
        membership()
    );
    assert_eq!(
        expect_present(joiner_task.await.expect("the joiner")),
        membership()
    );
    assert_eq!(rows.calls(), 1);
    drain(&tasks).await;
}

#[tokio::test]
async fn non_task_entry_does_not_join() {
    let cache = FakeCache::new(60.0_f64);
    let tasks = TaskTracker::new();
    let missing = lookup(Arc::clone(&cache), None, Arc::new(NonTaskIndex), &tasks);
    let error = expect_error(
        missing
            .get_team_membership(Some("user-1"), Some("team-1"))
            .await,
    );
    assert_eq!(error.to_string(), "No db connected");
    let rows = FakeRows::new(RowsAnswer::Found);
    let stored = lookup(
        Arc::clone(&cache),
        Some(Arc::clone(&rows)),
        Arc::new(NonTaskIndex),
        &tasks,
    );
    let found = stored
        .get_team_membership(Some("user-1"), Some("team-1"))
        .await;
    expect_present(found);
    assert_eq!(rows.calls(), 1);
    drain(&tasks).await;
}

#[tokio::test]
async fn no_database_after_miss() {
    let cache = FakeCache::new(60.0_f64);
    let tasks = TaskTracker::new();
    let subject = lookup(Arc::clone(&cache), None, index().1, &tasks);
    let error = expect_error(
        subject
            .get_team_membership(Some("user-1"), Some("team-1"))
            .await,
    );
    assert!(matches!(error, MembershipLoadError::NoDatabase));
    assert_eq!(cache.reads(), vec![(key("user-1", "team-1"), true)]);
    assert!(cache.writes().is_empty());
}

#[tokio::test]
async fn redis_hit_before_sql() {
    let cache = FakeCache::new(60.0_f64);
    cache.seed_redis(&key("user-1", "team-1"), stored_membership());
    let rows = FakeRows::new(RowsAnswer::Found);
    let tasks = TaskTracker::new();
    let subject = lookup(
        Arc::clone(&cache),
        Some(Arc::clone(&rows)),
        index().1,
        &tasks,
    );
    let found = subject
        .get_team_membership(Some("user-1"), Some("team-1"))
        .await;
    assert_eq!(expect_present(found), membership());
    assert_eq!(rows.calls(), 0);
    assert!(cache.writes().is_empty());
    assert_eq!(
        cache.reads(),
        vec![
            (key("user-1", "team-1"), true),
            (key("user-1", "team-1"), false)
        ]
    );
    drain(&tasks).await;
}

#[tokio::test]
async fn redis_negative_before_sql() {
    let cache = FakeCache::new(60.0_f64);
    cache.seed_redis(&key("user-1", "team-1"), sentinel());
    let rows = FakeRows::new(RowsAnswer::Found);
    let tasks = TaskTracker::new();
    let subject = lookup(
        Arc::clone(&cache),
        Some(Arc::clone(&rows)),
        index().1,
        &tasks,
    );
    let found = subject
        .get_team_membership(Some("user-1"), Some("team-1"))
        .await;
    expect_absent(found);
    assert_eq!(rows.calls(), 0);
    assert!(cache.writes().is_empty());
    drain(&tasks).await;
}

#[tokio::test]
async fn exact_composite_lookup_with_budget() {
    let harness = Harness::new();
    let table = RecordingTable::new(TableMode::Row);
    let found = read_membership(
        Arc::clone(&table),
        harness.context(1.5_f64),
        "user-7",
        "team-9",
    )
    .await
    .expect("the membership read")
    .expect("the stored membership");
    assert_eq!(found, membership());
    let request = table.request().expect("the recorded request");
    assert_eq!(
        request.filter,
        MembershipFilter {
            user_id: "user-7".to_owned(),
            team_id: "team-9".to_owned(),
        }
    );
    assert_eq!(
        request.include,
        Presence::Value(MembershipInclude {
            litellm_budget_table: true,
        })
    );
    drain(&harness.tasks).await;
}

#[tokio::test]
async fn membership_lookup_deadline_and_metric() {
    let harness = Harness::new();
    let table = RecordingTable::new(TableMode::Elapsed);
    assert!(!harness.tracker.stalled_within(30.0_f64));
    let failure = read_membership(
        Arc::clone(&table),
        harness.context(2.5_f64),
        "user-1",
        "team-1",
    )
    .await
    .expect_err("the lookup exceeds its deadline");
    assert!(
        harness.tracker.stalled_within(30.0_f64),
        "the stalled lookup is recorded"
    );
    let AuthReadError::Lookup(DbLookupError::Deadline(deadline)) = *failure.source else {
        panic!("the read reports its deadline");
    };
    assert_eq!(deadline.lookup, "team_membership");
    assert_eq!(deadline.deadline_seconds.to_bits(), 2.5_f64.to_bits());
    assert!(deadline.cause.is_some());
    drain(&harness.tasks).await;
}

#[tokio::test]
async fn negative_uses_management_ttl() {
    let cache = FakeCache::new(123.5_f64);
    let rows = FakeRows::new(RowsAnswer::Missing);
    let tasks = TaskTracker::new();
    let subject = lookup(
        Arc::clone(&cache),
        Some(Arc::clone(&rows)),
        index().1,
        &tasks,
    );
    let found = subject
        .get_team_membership(Some("user-1"), Some("team-1"))
        .await;
    expect_absent(found);
    assert_eq!(
        cache.writes(),
        vec![RecordedWrite {
            key: key("user-1", "team-1"),
            kind: WriteKind::Absent,
            ttl: Some(123.5_f64),
        }]
    );
    drain(&tasks).await;
}

#[tokio::test]
async fn positive_omits_ttl() {
    let cache = FakeCache::new(123.5_f64);
    let rows = FakeRows::new(RowsAnswer::Found);
    let tasks = TaskTracker::new();
    let subject = lookup(
        Arc::clone(&cache),
        Some(Arc::clone(&rows)),
        index().1,
        &tasks,
    );
    let found = subject
        .get_team_membership(Some("user-1"), Some("team-1"))
        .await;
    expect_present(found);
    assert_eq!(
        cache.writes(),
        vec![RecordedWrite {
            key: key("user-1", "team-1"),
            kind: WriteKind::Membership,
            ttl: None,
        }]
    );
    drain(&tasks).await;
}

#[tokio::test]
async fn optional_trace_inputs_unused() {
    let (logs, _guard) = Logs::capture();
    let cache = FakeCache::new(60.0_f64);
    let rows = FakeRows::new(RowsAnswer::Found);
    let tasks = TaskTracker::new();
    let subject = lookup(
        Arc::clone(&cache),
        Some(Arc::clone(&rows)),
        index().1,
        &tasks,
    );
    let found = subject
        .get_team_membership(Some("user-1"), Some("team-1"))
        .await;
    expect_present(found);
    assert_eq!(logs.text(), "");
    drain(&tasks).await;
}
