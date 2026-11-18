use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_auth::auth_cache::WriteOptions;
use betterlite_db::budget_reset_policy::cache_invalidation::GLOBAL_PROXY_SPEND_CACHE_KEY;
use betterlite_redis::request_batch::batches::RequestRedisBatches;
use betterlite_redis::request_batch::lock;
use betterlite_redis::request_batch::scope::ACTIVE_REQUEST_BATCHES;
use rust_decimal::Decimal;
use tokio_util::task::TaskTracker;
use tracing::Level;

use super::super::cache::ArmedSpendRead;
use super::super::{
    UPDATE_CACHE_READ_SLOT, UpdateInput, arm_cache_read, read_cache_values, update_spend_cache,
};
use super::support::{
    FakeCache, FakeState, HASHED_TEST_TOKEN, RecordingAlerts, capture_logs, in_request_scope,
    json_from, local_cache, unreachable_cache, update_deps, written,
};

fn armed(keys: &[String]) -> ArmedSpendRead {
    ArmedSpendRead {
        keys: keys.to_vec(),
        reads: Vec::new(),
    }
}

fn active_batches() -> Arc<RequestRedisBatches> {
    ACTIVE_REQUEST_BATCHES
        .try_with(Arc::clone)
        .expect("the request scope is active")
}

fn slot_is_armed() -> bool {
    ACTIVE_REQUEST_BATCHES
        .try_with(|batches| lock(&batches.prefetched).contains_key(UPDATE_CACHE_READ_SLOT))
        .expect("the request scope is active")
}

fn take_armed() -> ArmedSpendRead {
    let batches = active_batches();
    let slot = lock(&batches.prefetched)
        .remove(UPDATE_CACHE_READ_SLOT)
        .expect("the armed slot");
    *slot
        .downcast::<ArmedSpendRead>()
        .ok()
        .expect("the slot holds the armed read")
}

#[tokio::test]
async fn empty_keys_read_nothing() {
    let cache = FakeCache::new(FakeState::new());
    let values = read_cache_values(&[], None, cache.as_ref()).await;
    assert!(values.is_empty());
    assert!(cache.state().reads.is_empty());
}

#[tokio::test]
async fn the_read_map_keeps_only_cached_values() {
    let cache = FakeCache::new(
        FakeState::new().with_cached(BTreeMap::from([("user-1".to_owned(), json_from("5"))])),
    );
    let keys = vec!["user-1".to_owned(), GLOBAL_PROXY_SPEND_CACHE_KEY.to_owned()];
    let values = read_cache_values(&keys, None, cache.as_ref()).await;
    assert_eq!(values.len(), 1);
    assert_eq!(values.get("user-1"), Some(&json_from("5")));
}

#[tokio::test]
async fn a_failed_read_logs_the_warning_and_returns_no_values() {
    let cache = FakeCache::new(FakeState::new().failing_reads());
    let (log, _guard) = capture_logs(Level::WARN);
    let keys = vec!["user-1".to_owned()];
    let values = read_cache_values(&keys, None, cache.as_ref()).await;
    assert!(values.is_empty());
    let text = written(&log);
    assert!(text.contains(
        "Spend tracking - failed to read cached spend objects. Budget enforcement may use stale spend values."
    ));
}

#[tokio::test]
async fn a_failed_armed_resolve_logs_the_warning_and_returns_no_values() {
    let cache = FakeCache::new(FakeState::new().failing_resolve());
    let keys = vec!["user-1".to_owned()];
    let (log, _guard) = capture_logs(Level::WARN);
    let values = in_request_scope(async {
        let batches = active_batches();
        let _previous = lock(&batches.prefetched)
            .insert(UPDATE_CACHE_READ_SLOT.to_owned(), Box::new(armed(&keys)));
        read_cache_values(&keys, None, cache.as_ref()).await
    })
    .await;
    assert!(values.is_empty());
    assert_eq!(cache.state().resolves, 1);
    assert!(cache.state().reads.is_empty());
    let text = written(&log);
    assert!(text.contains(
        "Spend tracking - failed to read cached spend objects. Budget enforcement may use stale spend values."
    ));
}

#[tokio::test]
async fn a_failed_read_updates_nothing() {
    let cache = FakeCache::new(
        FakeState::new()
            .with_cached(BTreeMap::from([
                (
                    "user-1".to_owned(),
                    json_from(r#"{"user_id":"user-1","spend":1}"#),
                ),
                (GLOBAL_PROXY_SPEND_CACHE_KEY.to_owned(), json_from("10")),
            ]))
            .failing_reads(),
    );
    let alerts = Arc::new(RecordingAlerts::default());
    let tasks = TaskTracker::new();
    let deps = update_deps(Arc::clone(&cache), Arc::clone(&alerts), &tasks);
    let input = UpdateInput {
        token: None,
        user_id: Some("user-1".to_owned()),
        end_user_id: None,
        team_id: None,
        response_cost: Some(Decimal::from(5)),
        span: None,
        tags: None,
    };
    update_spend_cache(&input, &deps)
        .await
        .expect("the update runs");
    tasks.close();
    tasks.wait().await;
    assert!(cache.state().writes.is_empty());
}

#[tokio::test]
async fn the_armed_read_is_used_when_its_keys_match() {
    let cache = FakeCache::new(FakeState::new().with_cached(BTreeMap::from([
        ("user-1".to_owned(), json_from("5")),
        (GLOBAL_PROXY_SPEND_CACHE_KEY.to_owned(), json_from("10")),
    ])));
    let keys = vec!["user-1".to_owned(), GLOBAL_PROXY_SPEND_CACHE_KEY.to_owned()];
    let values = in_request_scope(async {
        let batches = active_batches();
        let _previous = lock(&batches.prefetched)
            .insert(UPDATE_CACHE_READ_SLOT.to_owned(), Box::new(armed(&keys)));
        read_cache_values(&keys, None, cache.as_ref()).await
    })
    .await;
    assert_eq!(values.len(), 2);
    let state = cache.state();
    assert_eq!(state.resolves, 1);
    assert!(state.reads.is_empty());
}

#[tokio::test]
async fn an_armed_read_with_other_keys_is_dropped_and_a_batch_read_runs() {
    let cache = FakeCache::new(
        FakeState::new().with_cached(BTreeMap::from([("user-1".to_owned(), json_from("5"))])),
    );
    let armed_keys = vec!["another-key".to_owned()];
    let read_keys = vec!["user-1".to_owned()];
    let (values, still_armed) = in_request_scope(async {
        let batches = active_batches();
        let _previous = lock(&batches.prefetched).insert(
            UPDATE_CACHE_READ_SLOT.to_owned(),
            Box::new(armed(&armed_keys)),
        );
        let values = read_cache_values(&read_keys, None, cache.as_ref()).await;
        (values, slot_is_armed())
    })
    .await;
    assert_eq!(values.len(), 1);
    assert!(!still_armed);
    let state = cache.state();
    assert_eq!(state.resolves, 0);
    assert_eq!(state.reads, [read_keys]);
}

#[tokio::test]
async fn an_armed_read_of_another_type_is_ignored() {
    let cache = FakeCache::new(FakeState::new());
    let read_keys = vec!["user-1".to_owned()];
    let values = in_request_scope(async {
        let batches = active_batches();
        let _previous =
            lock(&batches.prefetched).insert(UPDATE_CACHE_READ_SLOT.to_owned(), Box::new(7_u8));
        read_cache_values(&read_keys, None, cache.as_ref()).await
    })
    .await;
    assert!(values.is_empty());
    assert_eq!(cache.state().reads, [read_keys]);
}

#[test]
fn arming_outside_a_request_scope_does_nothing() {
    let cache = FakeCache::new(FakeState::new().armable());
    let keys = vec!["user-1".to_owned()];
    arm_cache_read(&keys, cache.as_ref()).expect("arming without a request batch is a no-op");
    assert!(cache.state().declared.is_empty());
}

#[tokio::test]
async fn arming_without_a_redis_tier_does_nothing() {
    let cache = FakeCache::new(FakeState::new());
    let keys = vec!["user-1".to_owned()];
    let armed = in_request_scope(async {
        arm_cache_read(&keys, cache.as_ref()).expect("arming without redis is a no-op");
        slot_is_armed()
    })
    .await;
    assert!(!armed);
    assert!(cache.state().declared.is_empty());
}

#[tokio::test]
async fn arming_without_keys_does_nothing() {
    let cache = FakeCache::new(FakeState::new().armable());
    let armed = in_request_scope(async {
        arm_cache_read(&[], cache.as_ref()).expect("arming without keys is a no-op");
        slot_is_armed()
    })
    .await;
    assert!(!armed);
    assert!(cache.state().declared.is_empty());
}

#[tokio::test]
async fn arming_declares_the_batch_read_under_the_slot() {
    let cache = FakeCache::new(FakeState::new().armable());
    let keys = vec!["user-1".to_owned(), GLOBAL_PROXY_SPEND_CACHE_KEY.to_owned()];
    let armed = in_request_scope(async {
        arm_cache_read(&keys, cache.as_ref()).expect("arming");
        take_armed()
    })
    .await;
    assert_eq!(armed.keys, keys);
    assert_eq!(cache.state().declared, [keys]);
}

#[tokio::test]
async fn a_real_cache_without_redis_does_not_arm() {
    let cache = local_cache();
    let keys = vec!["user-1".to_owned()];
    let armed = in_request_scope(async {
        arm_cache_read(&keys, &cache).expect("arming without redis is a no-op");
        slot_is_armed()
    })
    .await;
    assert!(!armed);
}

#[tokio::test]
async fn a_real_cache_with_redis_declares_the_batch_read() {
    let cache = unreachable_cache().await;
    let keys = vec!["user-1".to_owned(), GLOBAL_PROXY_SPEND_CACHE_KEY.to_owned()];
    let armed = in_request_scope(async {
        arm_cache_read(&keys, &cache).expect("arming");
        take_armed()
    })
    .await;
    assert_eq!(armed.keys, keys);
    assert_eq!(armed.reads.len(), 1);
}

#[tokio::test]
async fn a_real_cache_with_redis_arms_one_read_per_partition() {
    let cache = unreachable_cache().await;
    let keys = vec!["user-1".to_owned(), HASHED_TEST_TOKEN.to_owned()];
    let armed = in_request_scope(async {
        arm_cache_read(&keys, &cache).expect("arming");
        take_armed()
    })
    .await;
    assert_eq!(armed.keys, keys);
    assert_eq!(armed.reads.len(), 2);
}

#[tokio::test]
async fn the_real_cache_reads_the_cached_values_without_an_armed_read() {
    let cache = local_cache();
    cache
        .async_set(
            Some("user-1"),
            &json_from("5"),
            WriteOptions {
                ttl: None,
                local_only: true,
                refresh_ttl: false,
            },
        )
        .await
        .expect("the memory fixture is seeded");
    let keys = vec!["user-1".to_owned()];
    let values = read_cache_values(&keys, None, &cache).await;
    assert_eq!(values.len(), 1);
    assert_eq!(values.get("user-1"), Some(&json_from("5")));
}

#[tokio::test]
async fn the_real_armed_read_resolves_the_cached_values() {
    let cache = unreachable_cache().await;
    let keys = vec!["user-1".to_owned()];
    cache
        .async_set(
            Some("user-1"),
            &json_from("5"),
            WriteOptions {
                ttl: None,
                local_only: true,
                refresh_ttl: false,
            },
        )
        .await
        .expect("the memory fixture is seeded");
    let values = in_request_scope(async {
        arm_cache_read(&keys, &cache).expect("arming");
        read_cache_values(&keys, None, &cache).await
    })
    .await;
    assert_eq!(values.len(), 1);
    assert_eq!(values.get("user-1"), Some(&json_from("5")));
}
