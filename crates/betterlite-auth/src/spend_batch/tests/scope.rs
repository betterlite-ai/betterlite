use std::sync::Arc;

use betterlite_redis::request_batch::lock;
use tokio::task::JoinSet;

use crate::spend_batch::batch::{SpendCounterBatch, SpendCounterValue};
use crate::spend_batch::keys::admission_counter_keys;
use crate::spend_batch::scope::SpendBatchContext;

use super::support::{FakeRedis, Reply, allow_cache, decimal, keys, user_auth};

fn active(context: &SpendBatchContext) -> Option<Arc<SpendCounterBatch>> {
    lock(&context.active).clone()
}

#[tokio::test]
async fn a_scope_without_redis_does_nothing() {
    let context = SpendBatchContext::default();
    let scope = context.enter(None, &keys(&["spend:key:a"]));
    assert!(active(&context).is_none());
    context.bind_spend_counter_keys(&keys(&["spend:key:b"]));
    context.record_spend_counter_value("spend:key:a", decimal("1"));
    context.forget_spend_counter("spend:key:a");
    context.release_spend_counter_batch();
    assert_eq!(
        context
            .read_spend_counter("spend:key:a")
            .await
            .expect("the read succeeds"),
        None
    );
    assert_eq!(
        context
            .read_batched_spend_counter("spend:key:a")
            .await
            .expect("the read succeeds"),
        None
    );
    drop(scope);
    assert!(active(&context).is_none());
}

#[tokio::test]
async fn a_scope_opens_a_batch_with_its_keys() {
    let server = FakeRedis::start().await;
    let cache = allow_cache(&server).await;
    let context = SpendBatchContext::default();
    let scope = context.enter(Some(&cache), &keys(&["spend:key:b", "spend:key:a"]));
    let batch = active(&context).expect("the scope opens a batch");
    server.script_mget(Reply::Array(vec![
        Reply::Bulk("1".to_owned()),
        Reply::Bulk("2".to_owned()),
    ]));
    assert_eq!(
        batch.read("spend:key:a").await.expect("the read succeeds"),
        Some(SpendCounterValue::Amount(decimal("1")))
    );
    assert_eq!(
        server.mget_keys(),
        [vec!["spend:key:a".to_owned(), "spend:key:b".to_owned()]]
    );
    drop(scope);
    assert!(active(&context).is_none());
}

#[tokio::test]
async fn an_inner_scope_joins_the_open_outer_batch() {
    let server = FakeRedis::start().await;
    let cache = allow_cache(&server).await;
    let context = SpendBatchContext::default();
    let outer = context.enter(Some(&cache), &keys(&["spend:key:a"]));
    let outer_batch = active(&context).expect("the outer batch");
    {
        let _inner = context.enter(Some(&cache), &keys(&["spend:key:b"]));
        let inner_batch = active(&context).expect("the joined batch");
        assert!(Arc::ptr_eq(&outer_batch, &inner_batch));
    }
    let current = active(&context).expect("the outer batch stays active");
    assert!(Arc::ptr_eq(&outer_batch, &current));
    server.script_mget(Reply::Array(vec![
        Reply::Bulk("1".to_owned()),
        Reply::Bulk("2".to_owned()),
    ]));
    assert_eq!(
        current
            .read("spend:key:a")
            .await
            .expect("the read succeeds"),
        Some(SpendCounterValue::Amount(decimal("1")))
    );
    assert_eq!(
        server.mget_keys(),
        [vec!["spend:key:a".to_owned(), "spend:key:b".to_owned()]]
    );
    drop(outer);
    assert!(active(&context).is_none());
}

#[tokio::test]
async fn an_inner_scope_replaces_a_closed_outer_batch() {
    let server = FakeRedis::start().await;
    let cache = allow_cache(&server).await;
    let context = SpendBatchContext::default();
    let outer = context.enter(Some(&cache), &keys(&["spend:key:a"]));
    let outer_batch = active(&context).expect("the outer batch");
    context.release_spend_counter_batch();
    let inner = context.enter(Some(&cache), &keys(&["spend:key:b"]));
    let inner_batch = active(&context).expect("the inner batch");
    assert!(!Arc::ptr_eq(&outer_batch, &inner_batch));
    server.script_mget(Reply::Array(vec![Reply::Bulk("2".to_owned())]));
    assert_eq!(
        inner_batch
            .read("spend:key:b")
            .await
            .expect("the read succeeds"),
        Some(SpendCounterValue::Amount(decimal("2")))
    );
    drop(inner);
    let restored = active(&context).expect("the closed outer batch comes back");
    assert!(Arc::ptr_eq(&outer_batch, &restored));
    drop(outer);
    assert!(active(&context).is_none());
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
    assert!(active(&context).is_some());
    tasks.abort_all();
    assert!(tasks.join_next().await.expect("the task runs").is_err());
    assert!(active(&context).is_none());
}

#[tokio::test]
async fn release_closes_the_batch_for_reads_and_recorded_writes() {
    let server = FakeRedis::start().await;
    let cache = allow_cache(&server).await;
    let context = SpendBatchContext::default();
    let _scope = context.enter(Some(&cache), &keys(&["spend:key:a"]));
    context.release_spend_counter_batch();
    context.bind_spend_counter_keys(&keys(&["spend:key:b"]));
    context.record_spend_counter_value("spend:key:a", decimal("5"));
    assert_eq!(
        context
            .read_spend_counter("spend:key:a")
            .await
            .expect("the read succeeds"),
        None
    );
    assert_eq!(
        context
            .read_spend_counter("spend:key:b")
            .await
            .expect("the read succeeds"),
        None
    );
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
        context
            .read_spend_counter("spend:key:a")
            .await
            .expect("the read succeeds"),
        Some(SpendCounterValue::Amount(decimal("5")))
    );
    context.forget_spend_counter("spend:key:a");
    assert_eq!(
        context
            .read_spend_counter("spend:key:a")
            .await
            .expect("the read succeeds"),
        None
    );
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
            .await
            .expect("the read succeeds"),
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
    let expected: Vec<String> = admission_counter_keys(&auth, Some("end-1"))
        .into_iter()
        .collect();
    server.script_mget(Reply::Array(vec![Reply::Nil; expected.len()]));
    assert_eq!(
        context
            .read_spend_counter("spend:key:tok-1")
            .await
            .expect("the read succeeds"),
        Some(SpendCounterValue::Missing)
    );
    assert_eq!(server.mget_keys(), [expected]);
}
