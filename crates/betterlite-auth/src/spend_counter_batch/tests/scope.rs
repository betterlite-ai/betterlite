use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use betterlite_redis::request_batch::lock;
use tokio::task::JoinSet;

use crate::spend_counter_batch::batch::{BatchPhase, CounterValue, SpendCounterBatch};
use crate::spend_counter_batch::scope::SpendCounterScope;

use super::support::{FakeRedis, Reply, allow_cache, decimal, keys, user_auth};

fn active(scope: &SpendCounterScope) -> Arc<SpendCounterBatch> {
    lock(&scope.active)
        .clone()
        .expect("the scope holds a batch")
}

#[tokio::test]
async fn no_active_scope_wrappers() {
    let scope = SpendCounterScope::default();
    scope.bind_spend_counter_keys(&keys(&["spend:key:a"]));
    scope.record_spend_counter_value("spend:key:a", decimal("1"));
    scope.forget_spend_counter("spend:key:a");
    scope.release_spend_counter_batch();
    assert!(lock(&scope.active).is_none());
    assert!(
        scope
            .read_spend_counter("spend:key:a")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        scope
            .read_batched_spend_counter("spend:key:a")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn none_redis_preserves_outer() {
    let server = FakeRedis::start().await;
    let cache = allow_cache(&server).await;
    let scope = SpendCounterScope::default();
    let outer = scope.enter(Some(&cache), &keys(&["spend:key:a"]));
    let batch = active(&scope);
    let inner = scope.enter(None, &keys(&["spend:key:b"]));
    assert!(Arc::ptr_eq(&active(&scope), &batch));
    let state = lock(&batch.state);
    assert!(!state.keys.contains("spend:key:b"));
    drop(state);
    drop(inner);
    assert!(Arc::ptr_eq(&active(&scope), &batch));
    drop(outer);
    assert!(lock(&scope.active).is_none());
}

#[tokio::test]
async fn open_outer_reused_even_different_redis() {
    let first_server = FakeRedis::start().await;
    let second_server = FakeRedis::start().await;
    let first = allow_cache(&first_server).await;
    let second = allow_cache(&second_server).await;
    let scope = SpendCounterScope::default();
    let _outer = scope.enter(Some(&first), &keys(&["spend:key:a"]));
    let batch = active(&scope);
    let inner = scope.enter(Some(&second), &keys(&["spend:key:b"]));
    assert!(Arc::ptr_eq(&active(&scope), &batch));
    drop(inner);
    first_server.script_mget(Reply::Array(vec![
        Reply::Bulk("1".to_owned()),
        Reply::Bulk("2".to_owned()),
    ]));
    assert_eq!(
        scope.read_spend_counter("spend:key:b").await.unwrap(),
        Some(CounterValue::Amount(decimal("2")))
    );
    assert_eq!(first_server.commands_named("MGET").len(), 1);
    assert!(second_server.commands_named("MGET").is_empty());
}

#[tokio::test]
async fn closed_outer_gets_new_inner() {
    let first_server = FakeRedis::start().await;
    let second_server = FakeRedis::start().await;
    let first = allow_cache(&first_server).await;
    let second = allow_cache(&second_server).await;
    let scope = SpendCounterScope::default();
    let _outer = scope.enter(Some(&first), &keys(&["spend:key:a"]));
    let outer_batch = active(&scope);
    lock(&outer_batch.state).phase = BatchPhase::Closed;
    let inner = scope.enter(Some(&second), &keys(&["spend:key:b"]));
    let inner_batch = active(&scope);
    assert!(!Arc::ptr_eq(&inner_batch, &outer_batch));
    assert_eq!(lock(&inner_batch.state).phase, BatchPhase::Open);
    assert_eq!(lock(&outer_batch.state).phase, BatchPhase::Closed);
    drop(inner);
    assert!(Arc::ptr_eq(&active(&scope), &outer_batch));
}

#[tokio::test]
async fn exit_restores_on_success_and_failure() {
    let server = FakeRedis::start().await;
    let cache = allow_cache(&server).await;
    let scope = SpendCounterScope::default();
    let guard = scope.enter(Some(&cache), &keys(&["spend:key:a"]));
    assert!(lock(&scope.active).is_some());
    drop(guard);
    assert!(lock(&scope.active).is_none());
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        let _guard = scope.enter(Some(&cache), &keys(&["spend:key:a"]));
        panic!("the request failed");
    }));
    assert!(outcome.is_err());
    assert!(lock(&scope.active).is_none());
}

#[tokio::test]
async fn exit_does_not_close_batch() {
    let server = FakeRedis::start().await;
    let cache = allow_cache(&server).await;
    let scope = SpendCounterScope::default();
    let guard = scope.enter(Some(&cache), &keys(&["spend:key:a"]));
    let batch = active(&scope);
    drop(guard);
    assert!(lock(&scope.active).is_none());
    server.script_mget(Reply::Array(vec![Reply::Bulk("8".to_owned())]));
    assert_eq!(
        batch.read("spend:key:a").await.unwrap(),
        Some(CounterValue::Amount(decimal("8")))
    );
}

#[tokio::test]
async fn release_closes_only_active() {
    let first_server = FakeRedis::start().await;
    let second_server = FakeRedis::start().await;
    let first = allow_cache(&first_server).await;
    let second = allow_cache(&second_server).await;
    let scope = SpendCounterScope::default();
    let other = SpendCounterScope::default();
    let _outer = scope.enter(Some(&first), &keys(&["spend:key:a"]));
    let _other_guard = other.enter(Some(&second), &keys(&["spend:key:b"]));
    let batch = active(&scope);
    let other_batch = active(&other);
    scope.release_spend_counter_batch();
    assert_eq!(lock(&batch.state).phase, BatchPhase::Closed);
    assert_eq!(lock(&other_batch.state).phase, BatchPhase::Open);
}

#[tokio::test]
async fn request_state_isolated() {
    let first_server = FakeRedis::start().await;
    let second_server = FakeRedis::start().await;
    let first = allow_cache(&first_server).await;
    let second = allow_cache(&second_server).await;
    let request_a = Arc::new(SpendCounterScope::default());
    let request_b = Arc::new(SpendCounterScope::default());
    let _a = request_a.enter(Some(&first), &keys(&["spend:key:a"]));
    let _b = request_b.enter(Some(&second), &keys(&["spend:key:b"]));
    assert!(!Arc::ptr_eq(&active(&request_a), &active(&request_b)));
    first_server.script_mget(Reply::Array(vec![Reply::Bulk("6".to_owned())]));
    let mut tasks = JoinSet::new();
    let inherited = Arc::clone(&request_a);
    tasks.spawn(async move { inherited.read_spend_counter("spend:key:a").await });
    let value = tasks.join_next().await.unwrap().unwrap().unwrap();
    assert_eq!(value, Some(CounterValue::Amount(decimal("6"))));
    assert!(
        request_b
            .read_spend_counter("spend:key:a")
            .await
            .unwrap()
            .is_none()
    );
    assert!(second_server.commands_named("MGET").is_empty());
}

#[tokio::test]
async fn bind_on_read_adds_window_key() {
    let server = FakeRedis::start().await;
    let cache = allow_cache(&server).await;
    let scope = SpendCounterScope::default();
    let _guard = scope.enter(Some(&cache), &keys(&[]));
    server.script_mget(Reply::Array(vec![Reply::Bulk("3".to_owned())]));
    assert_eq!(
        scope
            .read_batched_spend_counter("spend:window:1")
            .await
            .unwrap(),
        Some(CounterValue::Amount(decimal("3")))
    );
    assert_eq!(server.mget_keys(), [vec!["spend:window:1".to_owned()]]);
}

#[tokio::test]
async fn admission_rebind_adds_new_ids() {
    let server = FakeRedis::start().await;
    let cache = allow_cache(&server).await;
    let scope = SpendCounterScope::default();
    let _guard = scope.enter(Some(&cache), &keys(&[]));
    let first = user_auth(&[("token", "token-1"), ("team_id", "team-1")]);
    scope.bind_admission_counter_keys(&first, None);
    let second = user_auth(&[
        ("token", "token-1"),
        ("team_id", "team-1"),
        ("user_id", "user-1"),
    ]);
    scope.bind_admission_counter_keys(&second, Some("end-1"));
    server.script_mget(Reply::Array(vec![Reply::Bulk("1".to_owned()); 5]));
    assert_eq!(
        scope.read_spend_counter("spend:key:token-1").await.unwrap(),
        Some(CounterValue::Amount(decimal("1")))
    );
    assert_eq!(
        server.mget_keys(),
        [[
            "spend:end_user:end-1",
            "spend:key:token-1",
            "spend:team:team-1",
            "spend:team_member:user-1:team-1",
            "spend:user:user-1",
        ]
        .map(str::to_owned)]
    );
}
