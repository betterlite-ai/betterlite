#![cfg(test)]

use std::sync::Arc;

use betterlite_redis::cache_primitives::protocol::RedisScalar;

use super::pipeline_scripts;
use crate::limiter_records::ScriptReply;
use crate::rate_limit_window_store::tests::support::{
    FakeBatch, FakeBatchSource, FakeCache, FakeScript, key_list, store_with,
};
use crate::rate_limit_window_store::{CounterScript, RedisCacheKind, ScriptCall, WindowStore};

fn window_args() -> Vec<RedisScalar> {
    vec![
        RedisScalar::Text("100".to_owned()),
        RedisScalar::Text("60".to_owned()),
    ]
}

fn calls() -> Vec<ScriptCall> {
    vec![
        (
            key_list(&["{key:value}:window", "{key:value}:requests"]),
            window_args(),
        ),
        (
            key_list(&["{other:value}:window", "{other:value}:requests"]),
            window_args(),
        ),
    ]
}

fn batched(reply: ScriptReply) -> (WindowStore, Arc<FakeBatch>) {
    let batch = FakeBatch::new(reply);
    let source = Arc::new(FakeBatchSource {
        batch: Some(Arc::clone(&batch)),
    });
    let mut store = store_with(FakeCache::new(Some(RedisCacheKind::Standalone)));
    store.request_batch = Some(source);
    (store, batch)
}

#[test]
fn no_open_batch_leaves_every_call_unbatched() {
    let store = store_with(FakeCache::new(Some(RedisCacheKind::Standalone)));
    let script: Arc<dyn CounterScript> = FakeScript::new("return {}", Vec::new());
    let handles = pipeline_scripts(&store, &script, &calls());
    assert_eq!(handles.len(), 2);
    assert!(handles.iter().all(Option::is_none));
}

#[test]
fn a_source_without_an_active_batch_declares_no_script_call() {
    let source = Arc::new(FakeBatchSource { batch: None });
    let mut store = store_with(FakeCache::new(Some(RedisCacheKind::Standalone)));
    store.request_batch = Some(source);
    let fake = FakeScript::new("return {}", Vec::new());
    let script: Arc<dyn CounterScript> = Arc::<FakeScript>::clone(&fake);
    let handles = pipeline_scripts(&store, &script, &calls());
    assert_eq!(handles.len(), 2);
    assert!(handles.iter().all(Option::is_none));
    assert!(fake.calls().is_empty());
}

#[test]
fn an_open_batch_declares_one_script_call_per_group() {
    let reply = ScriptReply::Array(vec![ScriptReply::Int(1), ScriptReply::Int(2)]);
    let (store, batch) = batched(reply);
    let script: Arc<dyn CounterScript> = FakeScript::new("return {}", Vec::new());
    let source = calls();
    let handles = pipeline_scripts(&store, &script, &source);
    assert!(handles.iter().all(Option::is_some));
    assert_eq!(batch.calls(), source);
}

#[tokio::test]
async fn a_declared_call_resolves_to_the_script_reply() {
    let reply = ScriptReply::Array(vec![ScriptReply::Int(1), ScriptReply::Int(2)]);
    let (store, _batch) = batched(reply.clone());
    let script: Arc<dyn CounterScript> = FakeScript::new("return {}", Vec::new());
    let handles = pipeline_scripts(&store, &script, &calls());
    let handle = handles
        .into_iter()
        .next()
        .flatten()
        .expect("a batch handle");
    assert_eq!(handle.await.expect("the declared reply"), reply);
}
