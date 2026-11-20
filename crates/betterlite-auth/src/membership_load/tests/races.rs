#![cfg(test)]

use std::future::pending;
use std::sync::Arc;

use betterlite_models::decisions::Json;
use rust_decimal::Decimal;
use tokio::sync::{oneshot, watch};
use tokio_util::task::TaskTracker;

use super::support::{
    CountingIndex, FakeCache, FakeRows, ResetRig, RowsAnswer, drain, expect_present, index, key,
    lookup, membership, reset_rig, wait_until_popped,
};
use crate::auth_invalidation::keys::InvalidationKey;
use crate::auth_invalidation::registry::InflightLoad;
use crate::membership_load::cache::MembershipCache;
use crate::membership_load::index::MembershipIndex;
use crate::membership_load::load::{LoadRequest, MembershipLoad, run_load};
use crate::membership_load::rows::TeamMembershipRows;

#[tokio::test]
async fn simultaneous_requests_share_load() {
    let cache = FakeCache::new(60.0_f64);
    let rows = FakeRows::blocked(RowsAnswer::Found);
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
    let second = lookup(
        Arc::clone(&cache),
        Some(Arc::clone(&rows)),
        index_handle,
        &tasks,
    );
    let second_task = tasks.spawn(async move {
        second
            .get_team_membership(Some("user-1"), Some("team-1"))
            .await
    });
    counting.wait_entry().await;
    rows.release();
    let first_found = first_task.await.expect("the first caller");
    let second_found = second_task.await.expect("the second caller");
    assert_eq!(expect_present(first_found), membership());
    assert_eq!(expect_present(second_found), membership());
    assert_eq!(rows.calls(), 1);
    assert_eq!(cache.writes().len(), 1);
    drain(&tasks).await;
}

#[tokio::test]
async fn waiter_cancel_keeps_load_running() {
    let cache = FakeCache::new(60.0_f64);
    let rows = FakeRows::blocked(RowsAnswer::Found);
    let tasks = TaskTracker::new();
    let subject = lookup(
        Arc::clone(&cache),
        Some(Arc::clone(&rows)),
        index().1,
        &tasks,
    );
    let waiter = tasks.spawn(async move {
        subject
            .get_team_membership(Some("user-1"), Some("team-1"))
            .await
    });
    rows.wait_started().await;
    waiter.abort();
    assert!(
        waiter
            .await
            .expect_err("the waiter was cancelled")
            .is_cancelled()
    );
    rows.release();
    drain(&tasks).await;
    assert!(rows.completed());
    assert_eq!(cache.writes().len(), 1);
}

#[tokio::test]
async fn completed_task_removed() {
    let cache = FakeCache::new(60.0_f64);
    let rows = FakeRows::new(RowsAnswer::Found);
    let (registry, shared) = index();
    let tasks = TaskTracker::new();
    let subject = lookup(Arc::clone(&cache), Some(Arc::clone(&rows)), shared, &tasks);
    let found = subject
        .get_team_membership(Some("user-1"), Some("team-1"))
        .await;
    expect_present(found);
    drain(&tasks).await;
    assert!(registry.get(&key("user-1", "team-1")).is_none());
}

#[tokio::test]
async fn replacement_survives_old_completion() {
    let cache = FakeCache::new(60.0_f64);
    let rows = FakeRows::blocked(RowsAnswer::Found);
    let (registry, shared) = index();
    let tasks = TaskTracker::new();
    let reservation = key("user-1", "team-1");
    let (start, started) = oneshot::channel();
    let (completion, receiver) = watch::channel(None::<MembershipLoad>);
    let rows_handle: Arc<dyn TeamMembershipRows> = Arc::clone(&rows);
    let cache_handle: Arc<dyn MembershipCache> = Arc::clone(&cache);
    let old = tasks.spawn(run_load(LoadRequest {
        start: started,
        completion,
        index: Arc::clone(&shared),
        key: reservation.clone(),
        user_id: "user-1".to_owned(),
        team_id: "team-1".to_owned(),
        rows: rows_handle,
        cache: cache_handle,
    }));
    start.send(()).expect("the load starts");
    rows.wait_started().await;
    shared.install(
        &reservation,
        InflightLoad {
            id: old.id(),
            completion: receiver,
        },
    );
    assert!(registry.pop(&reservation).is_some());
    let replacement = tasks.spawn(pending::<()>());
    let (_replacement_completion, replacement_receiver) = watch::channel(None::<MembershipLoad>);
    shared.install(
        &reservation,
        InflightLoad {
            id: replacement.id(),
            completion: replacement_receiver,
        },
    );
    rows.release();
    old.await.expect("the old load finishes");
    assert_eq!(
        registry.get(&reservation).map(|load| load.id),
        Some(replacement.id())
    );
    replacement.abort();
    drain(&tasks).await;
}

#[tokio::test]
async fn reset_waits_for_load_before_writes() {
    let cache = FakeCache::new(60.0_f64);
    let rows = FakeRows::blocked(RowsAnswer::Found);
    let (registry, shared) = index();
    let tasks = TaskTracker::new();
    let subject = lookup(Arc::clone(&cache), Some(Arc::clone(&rows)), shared, &tasks);
    let caller = tasks.spawn(async move {
        subject
            .get_team_membership(Some("user-1"), Some("team-1"))
            .await
    });
    rows.wait_started().await;
    let ResetRig {
        invalidation,
        spend,
    } = reset_rig();
    let counter = InvalidationKey::SpendCounter {
        user_id: "user-1",
        team_id: "team-1",
    }
    .render();
    assert!(spend.get_cache(&counter, true).is_none());
    let reset_registry = Arc::clone(&registry);
    let reset = tasks.spawn(async move {
        invalidation
            .invalidate_team_member_spend(
                &reset_registry,
                "user-1",
                "team-1",
                Some(Decimal::new(4, 0)),
            )
            .await
    });
    let reservation = key("user-1", "team-1");
    wait_until_popped(&registry, &reservation).await;
    assert!(
        registry.get(&reservation).is_none(),
        "the reset popped the load"
    );
    assert!(
        spend.get_cache(&counter, true).is_none(),
        "the reset writes nothing before the load finishes"
    );
    rows.release();
    assert_eq!(
        expect_present(caller.await.expect("the caller")),
        membership()
    );
    reset
        .await
        .expect("the reset task")
        .expect("the reset succeeds");
    let stored = spend.get_cache(&counter, true).expect("the reset counter");
    let Json::Number(number) = stored else {
        panic!("the spend counter is a number");
    };
    assert_eq!(number.as_f64(), Some(4.0_f64));
    drain(&tasks).await;
}

#[tokio::test]
async fn index_eviction_keeps_task_tracked() {
    let cache = FakeCache::new(60.0_f64);
    let rows = FakeRows::blocked(RowsAnswer::Found);
    let (registry, shared) = index();
    let tasks = TaskTracker::new();
    let subject = lookup(Arc::clone(&cache), Some(Arc::clone(&rows)), shared, &tasks);
    let caller = tasks.spawn(async move {
        subject
            .get_team_membership(Some("user-1"), Some("team-1"))
            .await
    });
    rows.wait_started().await;
    assert!(registry.pop(&key("user-1", "team-1")).is_some());
    rows.release();
    assert_eq!(
        expect_present(caller.await.expect("the caller")),
        membership()
    );
    drain(&tasks).await;
    assert!(rows.completed());
    assert_eq!(cache.writes().len(), 1);
}

#[tokio::test]
async fn shutdown_drains_loads() {
    let cache = FakeCache::new(60.0_f64);
    let rows = FakeRows::blocked(RowsAnswer::Found);
    let tasks = TaskTracker::new();
    let subject = lookup(
        Arc::clone(&cache),
        Some(Arc::clone(&rows)),
        index().1,
        &tasks,
    );
    let caller = tasks.spawn(async move {
        subject
            .get_team_membership(Some("user-1"), Some("team-1"))
            .await
    });
    rows.wait_started().await;
    caller.abort();
    assert!(
        caller
            .await
            .expect_err("the caller was cancelled")
            .is_cancelled()
    );
    rows.release();
    drain(&tasks).await;
    assert!(rows.completed());
    assert_eq!(cache.writes().len(), 1);
}
