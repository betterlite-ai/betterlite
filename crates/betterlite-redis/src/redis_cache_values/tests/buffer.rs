use std::collections::HashMap;
use std::future::Future as _;
use std::task::{Context, Waker};
use std::time::Duration;

use futures_util::future::join_all;
use tokio::time;

use super::support::{
    TestGate, harness, harness_on_transport, harness_with_gate, harness_without_server,
    sentinel_transport, settings,
};
use crate::redis_cache_values::error::RedisCacheValuesError;
use crate::redis_live_client::tests::fake_server::{FakeServer, Reply};

#[tokio::test]
async fn buffers_namespaced_entries_below_the_flush_size() {
    let server = FakeServer::start().await;
    let harness = harness(&server, settings(Some("ns"), Some(60), Some(3)));
    harness
        .values
        .batch_cache_write("alpha", "first")
        .await
        .expect("the first buffered write");
    harness
        .values
        .batch_cache_write("beta", "second")
        .await
        .expect("the second buffered write");
    assert!(server.commands().is_empty());
    assert_eq!(harness.values.buffered_count(), 2);
    assert_eq!(
        harness.gate.entries(),
        vec![
            "enter:batch_cache_write".to_owned(),
            "exit:1".to_owned(),
            "enter:batch_cache_write".to_owned(),
            "exit:1".to_owned()
        ]
    );
}

#[tokio::test]
async fn flushes_at_the_flush_size_with_the_default_ttl() {
    let server = FakeServer::start().await;
    let harness = harness(&server, settings(Some("ns"), Some(60), Some(2)));
    harness
        .values
        .batch_cache_write("alpha", "first")
        .await
        .expect("the first buffered write");
    harness
        .values
        .batch_cache_write("beta", "second")
        .await
        .expect("the flush write");
    let sets = server.commands_named("SET");
    assert_eq!(sets.len(), 2);
    assert_eq!(sets[0].args, ["SET", "ns:alpha", "\"first\"", "EX", "60"]);
    assert_eq!(sets[1].args, ["SET", "ns:beta", "\"second\"", "EX", "60"]);
    assert_eq!(harness.values.buffered_count(), 0);
}

#[tokio::test]
async fn keeps_entries_appended_during_the_flush() {
    let server = FakeServer::start().await;
    let harness = harness(&server, settings(Some("ns"), Some(60), Some(10)));
    harness
        .values
        .batch_cache_write("alpha", "first")
        .await
        .expect("the buffered write");
    let mut flush = Box::pin(harness.values.flush_cache_buffer());
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    assert!(
        flush.as_mut().poll(&mut context).is_pending(),
        "the flush waits on the pipeline"
    );
    harness
        .values
        .batch_cache_write("beta", "second")
        .await
        .expect("the write during the flush");
    assert!(
        time::timeout(Duration::from_secs(1), &mut flush)
            .await
            .is_ok(),
        "the flush completes"
    );
    assert_eq!(
        harness.values.buffered_count(),
        1,
        "the entry appended during the flush stays"
    );
    let sets = server.commands_named("SET");
    assert_eq!(sets.len(), 1);
    assert_eq!(sets[0].args, ["SET", "ns:alpha", "\"first\"", "EX", "60"]);
}

#[tokio::test]
async fn keeps_the_buffer_when_the_flush_returns_an_error() {
    let server = FakeServer::start().await;
    let harness = harness_with_gate(
        &server,
        settings(Some("ns"), Some(60), Some(1)),
        TestGate::named("async_set_cache_pipeline"),
    );
    let error = harness
        .values
        .batch_cache_write("alpha", "first")
        .await
        .expect_err("the open breaker rejects the flush");
    assert!(matches!(error, RedisCacheValuesError::BreakerOpen(_)));
    assert_eq!(harness.values.buffered_count(), 1);
    assert!(server.commands().is_empty());
    let gate = harness.gate.entries();
    assert_eq!(gate.len(), 3);
    assert_eq!(gate[0], "enter:batch_cache_write");
    assert_eq!(gate[1], "enter:async_set_cache_pipeline");
    assert!(gate[2].starts_with("failure:"));
    assert!(!gate.iter().any(|entry| entry.starts_with("exit")));
}

#[tokio::test]
async fn drops_the_entries_when_the_flush_swallows_an_error() {
    let server = FakeServer::start().await;
    server.script("SET", Reply::Error("ERR broken".to_owned()));
    let harness = harness(&server, settings(Some("ns"), Some(60), Some(1)));
    harness
        .values
        .batch_cache_write("alpha", "first")
        .await
        .expect("the swallowed flush failure");
    assert_eq!(harness.values.buffered_count(), 0);
    assert_eq!(server.commands_named("SET").len(), 1);
}

#[tokio::test]
async fn drops_the_entries_when_the_flush_cannot_connect() {
    let harness = harness_without_server(settings(Some("ns"), None, Some(1)));
    harness
        .values
        .batch_cache_write("alpha", "first")
        .await
        .expect("the swallowed connection failure");
    assert_eq!(harness.values.buffered_count(), 0);
}

#[tokio::test]
async fn drops_the_entries_when_the_flush_cannot_resolve_the_master() {
    let harness = harness_on_transport(settings(Some("ns"), None, Some(1)), sentinel_transport());
    harness
        .values
        .batch_cache_write("alpha", "first")
        .await
        .expect("the swallowed sentinel failure");
    assert_eq!(harness.values.buffered_count(), 0);
}

#[tokio::test]
async fn flushes_at_one_hundred_when_no_size_is_given() {
    let server = FakeServer::start().await;
    let harness = harness(&server, settings(Some("ns"), None, None));
    let keys: Vec<String> = (0_i32..99_i32)
        .map(|index| format!("key-{index}"))
        .collect();
    let writes = keys
        .iter()
        .map(|key| harness.values.batch_cache_write(key, "value"));
    let completed = join_all(writes).await;
    assert!(completed.iter().all(Result::is_ok));
    assert_eq!(completed.len(), 99);
    assert!(server.commands().is_empty());
    assert_eq!(harness.values.buffered_count(), 99);
    harness
        .values
        .batch_cache_write("key-99", "value")
        .await
        .expect("the flush write");
    assert_eq!(server.commands_named("SET").len(), 100);
    assert_eq!(harness.values.buffered_count(), 0);
}

#[tokio::test]
async fn fails_the_write_when_the_value_cannot_serialize() {
    let server = FakeServer::start().await;
    let harness = harness(&server, settings(Some("ns"), None, Some(2)));
    let map = HashMap::from([((1_u8, 2_u8), "value")]);
    let error = harness
        .values
        .batch_cache_write("alpha", &map)
        .await
        .expect_err("a tuple key cannot be a JSON object key");
    assert!(matches!(error, RedisCacheValuesError::Serialize(_)));
    assert_eq!(harness.values.buffered_count(), 0);
    assert!(server.commands().is_empty());
    let gate = harness.gate.entries();
    assert_eq!(gate[0], "enter:batch_cache_write");
    assert!(gate[1].starts_with("failure:"));
    assert_eq!(gate.len(), 2);
}
