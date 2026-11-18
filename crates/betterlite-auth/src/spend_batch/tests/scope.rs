use std::sync::Arc;

use betterlite_redis::request_batch::lock;
use tokio::task::JoinSet;

use crate::spend_batch::batch::SpendCounterValue;
use crate::spend_batch::keys::admission_counter_keys;
use crate::spend_batch::scope::SpendBatchContext;

use super::support::{FakeRedis, Reply, allow_cache, decimal, keys, user_auth};

#[tokio::test]
async fn a_scope_without_redis_does_nothing() {
    let context = SpendBatchContext::default();
    let scope = context.enter(None, &keys(&["spend:key:a"]));
    assert!(context.active_spend_counter_batch().is_none());
    context.bind_spend_counter_keys(&keys(&["spend:key:b"]));
    context.record_spend_counter_value("spend:key:a", decimal("1"));
    context.forget_spend_counter("spend:key:a");
    context.release_spend_counter_batch();
    assert_eq!(context.read_spend_counter("spend:key:a").await, None);
    assert_eq!(
        context.read_batched_spend_counter("spend:key:a").await,
        None
    );
    drop(scope);
    assert!(context.active_spend_counter_batch().is_none());
}

#[tokio::test]
async fn a_scope_opens_a_batch_with_its_keys() {
    let server = FakeRedis::start().await;
    let cache = allow_cache(&server).await;
    let context = SpendBatchContext::default();
    let scope = context.enter(Some(&cache), &keys(&["spend:key:b", "spend:key:a"]));
    let batch = context
        .active_spend_counter_batch()
        .expect("the scope opens a batch");
    assert!(batch.is_open());
    assert_eq!(
        lock(&batch.state).keys,
        keys(&["spend:key:a", "spend:key:b"])
    );
    drop(scope);
    assert!(context.active_spend_counter_batch().is_none());
}

#[tokio::test]
async fn an_inner_scope_joins_the_open_outer_batch() {
    let server = FakeRedis::start().await;
    let cache = allow_cache(&server).await;
    let context = SpendBatchContext::default();
    let outer = context.enter(Some(&cache), &keys(&["spend:key:a"]));
    let outer_batch = context
        .active_spend_counter_batch()
        .expect("the outer batch");
    {
        let _inner = context.enter(Some(&cache), &keys(&["spend:key:b"]));
        let inner_batch = context
            .active_spend_counter_batch()
            .expect("the joined batch");
        assert!(Arc::ptr_eq(&outer_batch, &inner_batch));
    }
    let current = context
        .active_spend_counter_batch()
        .expect("the outer batch stays active");
    assert!(Arc::ptr_eq(&outer_batch, &current));
    assert_eq!(
        lock(&outer_batch.state).keys,
        keys(&["spend:key:a", "spend:key:b"])
    );
    drop(outer);
    assert!(context.active_spend_counter_batch().is_none());
}

#[tokio::test]
async fn an_inner_scope_replaces_a_closed_outer_batch() {
    let server = FakeRedis::start().await;
    let cache = allow_cache(&server).await;
    let context = SpendBatchContext::default();
    let outer = context.enter(Some(&cache), &keys(&["spend:key:a"]));
    let outer_batch = context
        .active_spend_counter_batch()
        .expect("the outer batch");
    context.release_spend_counter_batch();
    let inner = context.enter(Some(&cache), &keys(&["spend:key:b"]));
    let inner_batch = context
        .active_spend_counter_batch()
        .expect("the inner batch");
    assert!(!Arc::ptr_eq(&outer_batch, &inner_batch));
    assert!(inner_batch.is_open());
    assert_eq!(lock(&inner_batch.state).keys, keys(&["spend:key:b"]));
    drop(inner);
    let restored = context
        .active_spend_counter_batch()
        .expect("the closed outer batch comes back");
    assert!(Arc::ptr_eq(&outer_batch, &restored));
    drop(outer);
    assert!(context.active_spend_counter_batch().is_none());
}

#[tokio::test]
async fn the_scope_restores_the_context_after_cancellation() {
    let server = FakeRedis::start().await;
    let cache = allow_cache(&server).await;
    let context = Arc::new(SpendBatchContext::default());
    let (started, arrived) = tokio::sync::oneshot::channel();
    let mut tasks = JoinSet::new();
    let scoped = Arc::clone(&context);
    tasks.spawn(async move {
        let _scope = scoped.enter(Some(&cache), &keys(&["spend:key:a"]));
        let _sent = started.send(());
        std::future::pending::<()>().await;
    });
    arrived.await.expect("the scoped task starts");
    assert!(context.active_spend_counter_batch().is_some());
    tasks.abort_all();
    assert!(tasks.join_next().await.expect("the task runs").is_err());
    assert!(context.active_spend_counter_batch().is_none());
}

#[tokio::test]
async fn release_closes_the_batch_for_reads_and_binds() {
    let server = FakeRedis::start().await;
    let cache = allow_cache(&server).await;
    let context = SpendBatchContext::default();
    let _scope = context.enter(Some(&cache), &keys(&["spend:key:a"]));
    context.release_spend_counter_batch();
    let batch = context
        .active_spend_counter_batch()
        .expect("the released batch");
    assert!(!batch.is_open());
    context.bind_spend_counter_keys(&keys(&["spend:key:b"]));
    assert!(!lock(&batch.state).keys.contains("spend:key:b"));
    assert_eq!(context.read_spend_counter("spend:key:a").await, None);
    assert!(server.commands_named("MGET").is_empty());
}

#[tokio::test]
async fn recorded_and_forgotten_values_are_visible_through_the_scope() {
    let server = FakeRedis::start().await;
    let cache = allow_cache(&server).await;
    let context = SpendBatchContext::default();
    let _scope = context.enter(Some(&cache), &keys(&[]));
    context.record_spend_counter_value("spend:key:a", decimal("5"));
    assert_eq!(
        context.read_spend_counter("spend:key:a").await,
        Some(SpendCounterValue::Amount(decimal("5")))
    );
    context.forget_spend_counter("spend:key:a");
    assert_eq!(context.read_spend_counter("spend:key:a").await, None);
    assert!(server.commands_named("MGET").is_empty());
}

#[tokio::test]
async fn a_batched_read_binds_its_key_on_demand() {
    let server = FakeRedis::start().await;
    let cache = allow_cache(&server).await;
    let context = SpendBatchContext::default();
    let _scope = context.enter(Some(&cache), &keys(&[]));
    server.script_mget(Reply::Array(vec![Reply::Bulk("4".to_owned())]));
    assert_eq!(
        context
            .read_batched_spend_counter("spend:budget_window:1")
            .await,
        Some(SpendCounterValue::Amount(decimal("4")))
    );
    assert_eq!(
        server.mget_keys(),
        [vec!["spend:budget_window:1".to_owned()]]
    );
}

#[tokio::test]
async fn admission_keys_are_bound_through_the_scope() {
    let server = FakeRedis::start().await;
    let cache = allow_cache(&server).await;
    let context = SpendBatchContext::default();
    let _scope = context.enter(Some(&cache), &keys(&[]));
    let auth = user_auth(&[
        ("token", "tok-1"),
        ("team_id", "team-1"),
        ("user_id", "user-1"),
    ]);
    context.bind_admission_counter_keys(&auth, Some("end-1"));
    let batch = context
        .active_spend_counter_batch()
        .expect("the active batch");
    assert_eq!(
        lock(&batch.state).keys,
        admission_counter_keys(&auth, Some("end-1"))
    );
}
