use crate::redis_client_reuse::transport::RedisCacheFamily;
use crate::redis_counters::RedisCountersError;
use crate::redis_live_client::tests::fake_server::{FakeServer, Reply};

use super::support::{
    cluster_pair, counters, direct_settings, key_in_slot_range, runtime, transport,
};

#[test]
fn batch_get_counts_returns_counts_and_misses_in_input_order() {
    let runtime = runtime();
    let server = runtime.block_on(FakeServer::start());
    server.script(
        "MGET",
        Reply::Array(vec![
            Reply::Bulk("3".to_owned()),
            Reply::Nil,
            Reply::Bulk("7".to_owned()),
        ]),
    );
    let (counters, _hooks, _gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        None,
    );
    let counts = counters
        .batch_get_counts(&["alpha".to_owned(), "missing".to_owned(), "gamma".to_owned()])
        .expect("the counts");
    assert_eq!(counts, vec![Some(3), None, Some(7)]);
    assert_eq!(
        server.commands_named("MGET")[0].args,
        ["MGET", "alpha", "missing", "gamma"]
    );
}

#[tokio::test]
async fn async_batch_get_counts_returns_counts_and_misses_in_input_order() {
    let server = FakeServer::start().await;
    server.script(
        "MGET",
        Reply::Array(vec![
            Reply::Bulk("3".to_owned()),
            Reply::Nil,
            Reply::Bulk("7".to_owned()),
        ]),
    );
    let (counters, _hooks, _gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        None,
    );
    let counts = counters
        .async_batch_get_counts(&["alpha".to_owned(), "missing".to_owned(), "gamma".to_owned()])
        .await
        .expect("the counts");
    assert_eq!(counts, vec![Some(3), None, Some(7)]);
    assert_eq!(
        server.commands_named("MGET")[0].args,
        ["MGET", "alpha", "missing", "gamma"]
    );
}

#[tokio::test]
async fn a_non_numeric_count_is_an_error_and_never_an_empty_list() {
    let server = FakeServer::start().await;
    server.script(
        "MGET",
        Reply::Array(vec![Reply::Bulk("not-a-count".to_owned())]),
    );
    let (counters, _hooks, gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        None,
    );
    let error = counters
        .async_batch_get_counts(&["alpha".to_owned()])
        .await
        .expect_err("the undecodable count");
    assert!(matches!(error, RedisCountersError::Decode(_)));
    assert_eq!(gate.failures().len(), 1);
    assert!(gate.exits().is_empty());
}

#[test]
fn a_server_error_is_returned_and_is_not_an_empty_list() {
    let runtime = runtime();
    let server = runtime.block_on(FakeServer::start());
    server.script("MGET", Reply::Error("ERR mget failed".to_owned()));
    let (counters, _hooks, _gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        None,
    );
    let error = counters
        .batch_get_counts(&["alpha".to_owned()])
        .expect_err("the server error");
    assert!(error.to_string().contains("mget failed"), "{error}");
}

#[test]
fn an_empty_count_list_sends_nothing() {
    let runtime = runtime();
    let server = runtime.block_on(FakeServer::start());
    let (counters, _hooks, _gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        None,
    );
    let counts = counters.batch_get_counts(&[]).expect("the counts");
    assert!(counts.is_empty());
    assert!(server.commands().is_empty());
}

#[tokio::test]
async fn an_empty_async_count_list_sends_nothing() {
    let server = FakeServer::start().await;
    let (counters, _hooks, _gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        None,
    );
    let counts = counters
        .async_batch_get_counts(&[])
        .await
        .expect("the counts");
    assert!(counts.is_empty());
    assert!(server.commands().is_empty());
}

#[tokio::test]
async fn count_keys_are_namespaced() {
    let server = FakeServer::start().await;
    server.script(
        "MGET",
        Reply::Array(vec![Reply::Bulk("1".to_owned()), Reply::Nil]),
    );
    let (counters, _hooks, _gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        Some("ns"),
    );
    let counts = counters
        .async_batch_get_counts(&["alpha".to_owned(), "ns:beta".to_owned()])
        .await
        .expect("the counts");
    assert_eq!(counts, vec![Some(1), None]);
    assert_eq!(
        server.commands_named("MGET")[0].args,
        ["MGET", "ns:alpha", "ns:beta"]
    );
}

#[tokio::test]
async fn two_cluster_nodes_still_give_input_order() {
    let pair = cluster_pair().await;
    let low = key_in_slot_range(0, 8191);
    let high = key_in_slot_range(8192, 16383);
    pair.first
        .script("MGET", Reply::Array(vec![Reply::Bulk("3".to_owned())]));
    pair.second
        .script("MGET", Reply::Array(vec![Reply::Bulk("7".to_owned())]));
    let (counters, _hooks, _gate) = counters(pair.transport, None);
    let counts = counters
        .async_batch_get_counts(&[low.to_owned(), high.to_owned()])
        .await
        .expect("the counts");
    assert_eq!(counts, vec![Some(3), Some(7)]);
    assert_eq!(pair.first.commands_named("MGET").len(), 1);
    assert_eq!(pair.second.commands_named("MGET").len(), 1);
}
