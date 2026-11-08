use std::future::Future as _;
use std::task::{Context, Waker};
use std::time::Duration;

use tokio::time;

use super::support::{TestGate, harness, harness_with_gate};
use crate::redis_cache_values::error::RedisCacheValuesError;
use crate::redis_live_client::tests::fake_server::{FakeServer, Reply};

#[tokio::test]
async fn buffers_namespaced_entries_below_the_flush_size() {
    let server = FakeServer::start().await;
    let harness = harness(&server, Some("ns"), Some(60), Some(3));
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
    let harness = harness(&server, Some("ns"), Some(60), Some(2));
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
    let harness = harness(&server, Some("ns"), Some(60), Some(10));
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
        Some("ns"),
        Some(60),
        Some(1),
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
}

#[tokio::test]
async fn drops_the_entries_when_the_flush_swallows_an_error() {
    let server = FakeServer::start().await;
    server.script("SET", Reply::Error("ERR broken".to_owned()));
    let harness = harness(&server, Some("ns"), Some(60), Some(1));
    harness
        .values
        .batch_cache_write("alpha", "first")
        .await
        .expect("the swallowed flush failure");
    assert_eq!(harness.values.buffered_count(), 0);
    assert_eq!(server.commands_named("SET").len(), 1);
}
