#![cfg(test)]

use std::sync::Arc;

use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_db::postgres_store::errors::DbError;
use tokio_util::task::TaskTracker;

use super::support::{
    CountingIndex, FakeCache, FakeRows, RowsAnswer, drain, expect_error, index, lookup, membership,
    shared, stored_membership,
};
use crate::auth_read::table::AuthReadError;
use crate::membership_load::error::MembershipLoadError;
use crate::membership_load::index::MembershipIndex;
use crate::membership_load::load::{MembershipLoad, shared_result};

#[tokio::test]
async fn local_read_error_propagates() {
    let cache = FakeCache::new(60.0_f64);
    cache.fail_reads(true);
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
    let error = expect_error(found);
    assert!(matches!(error, MembershipLoadError::CacheRead(_)));
    assert_eq!(rows.calls(), 0);
}

#[tokio::test]
async fn redis_read_error_propagates() {
    let cache = FakeCache::new(60.0_f64);
    cache.fail_reads(false);
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
    let error = shared(expect_error(found));
    assert!(matches!(&*error, MembershipLoadError::CacheRead(_)));
    assert_eq!(rows.calls(), 0);
    drain(&tasks).await;
}

#[tokio::test]
async fn database_error_does_not_cache_negative() {
    let cache = FakeCache::new(60.0_f64);
    let rows = FakeRows::new(RowsAnswer::Database);
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
    let error = shared(expect_error(found));
    assert!(matches!(&*error, MembershipLoadError::Read(_)));
    assert!(cache.writes().is_empty());
    assert_eq!(rows.calls(), 1);
    drain(&tasks).await;
}

#[tokio::test]
async fn row_validation_does_not_cache_negative() {
    let cache = FakeCache::new(60.0_f64);
    let rows = FakeRows::new(RowsAnswer::Validation);
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
    let error = shared(expect_error(found));
    let MembershipLoadError::Read(failure) = &*error else {
        panic!("the shared failure carries the row failure");
    };
    let AuthReadError::Repository(DbError::InvalidRecord { .. }) = failure.source.as_ref() else {
        panic!("the row validation failure is a repository record error");
    };
    assert!(cache.writes().is_empty());
    drain(&tasks).await;
}

#[rstest::rstest]
#[case::found(RowsAnswer::Found)]
#[case::missing(RowsAnswer::Missing)]
#[tokio::test]
async fn cache_write_failure_prevents_return(#[case] answer: RowsAnswer) {
    let cache = FakeCache::new(60.0_f64);
    cache.fail_writes();
    let rows = FakeRows::new(answer);
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
    let error = shared(expect_error(found));
    assert!(matches!(&*error, MembershipLoadError::CacheWrite(_)));
    assert_eq!(rows.calls(), 1);
    drain(&tasks).await;
}

#[tokio::test]
async fn all_waiters_observe_failure() {
    let cache = FakeCache::new(60.0_f64);
    let rows = FakeRows::blocked(RowsAnswer::Database);
    let shared_index = index().1;
    let counting = CountingIndex::new(Arc::clone(&shared_index));
    let index_handle: Arc<dyn MembershipIndex> = Arc::clone(&counting);
    let tasks = TaskTracker::new();
    let first = lookup(
        Arc::clone(&cache),
        Some(Arc::clone(&rows)),
        Arc::clone(&index_handle),
        &tasks,
    );
    let first_task = tasks.spawn(async move {
        first
            .get_team_membership(Some("user-1"), Some("team-1"))
            .await
    });
    rows.wait_started().await;
    counting.wait_entry().await;
    let second = lookup(Arc::clone(&cache), None, index_handle, &tasks);
    let second_task = tasks.spawn(async move {
        second
            .get_team_membership(Some("user-1"), Some("team-1"))
            .await
    });
    counting.wait_entry().await;
    rows.release();
    let first_error = shared(expect_error(first_task.await.expect("the first caller")));
    let second_error = shared(expect_error(second_task.await.expect("the second caller")));
    assert!(Arc::ptr_eq(&first_error, &second_error));
    assert_eq!(rows.calls(), 1);
    drain(&tasks).await;
}

#[test]
fn shared_projection_rejects_dictionary() {
    let stored = stored_membership();
    let text = serde_json::to_string(&stored).expect("the payload serializes");
    let value: CacheValue = serde_json::from_str(&text).expect("the payload converts");
    let rejected = shared_result(MembershipLoad::Foreign(Box::new(value)));
    assert_eq!(rejected.expect("the projection answers"), None);
    let typed = shared_result(MembershipLoad::Present(Box::new(membership())));
    assert_eq!(typed.expect("the projection answers"), Some(membership()));
}
