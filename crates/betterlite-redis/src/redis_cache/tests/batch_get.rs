use std::collections::BTreeMap;

use super::support::{harness, harness_at};
use crate::cache_primitives::decode::CachedValue;
use crate::redis_live_client::tests::fake_server::{FakeServer, Reply};

fn number(value: i64) -> CachedValue {
    CachedValue::Number(serde_json::Number::from(value))
}

fn object() -> CachedValue {
    CachedValue::Object(BTreeMap::from([("answer".to_owned(), number(1))]))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reads_the_namespaced_keys_with_one_mget_and_maps_the_original_keys() {
    let server = FakeServer::start().await;
    server.script(
        "MGET",
        Reply::Array(vec![Reply::Bulk("{\"answer\": 1}".to_owned()), Reply::Nil]),
    );
    let harness = harness(&server, Some("ns"), 60).await;
    let cached = harness
        .cache
        .async_batch_get_cache(&[Some("alpha"), Some("missing")])
        .await
        .expect("the batch read");
    assert_eq!(cached.len(), 2);
    assert_eq!(cached.get("alpha"), Some(&Some(object())));
    assert_eq!(cached.get("missing"), Some(&None));
    let mgets = server.commands_named("MGET");
    assert_eq!(mgets.len(), 1);
    assert_eq!(mgets[0].args, ["MGET", "ns:alpha", "ns:missing"]);
    assert_eq!(
        harness.hooks.for_call("async_batch_get_cache"),
        ["success:async_batch_get_cache:None"]
    );
    assert_eq!(
        harness.gate.entries(),
        ["enter:async_batch_get_cache", "exit:1"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn drops_none_keys() {
    let server = FakeServer::start().await;
    server.script(
        "MGET",
        Reply::Array(vec![Reply::Nil, Reply::Bulk("2".to_owned())]),
    );
    let harness = harness(&server, Some("ns"), 60).await;
    let cached = harness
        .cache
        .async_batch_get_cache(&[Some("alpha"), None, Some("beta")])
        .await
        .expect("the batch read");
    assert_eq!(
        server.commands_named("MGET")[0].args,
        ["MGET", "ns:alpha", "ns:beta"]
    );
    assert_eq!(cached.len(), 2);
    assert_eq!(cached.get("alpha"), Some(&None));
    assert_eq!(cached.get("beta"), Some(&Some(number(2))));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_duplicate_key_keeps_the_last_reply() {
    let server = FakeServer::start().await;
    server.script(
        "MGET",
        Reply::Array(vec![
            Reply::Bulk("1".to_owned()),
            Reply::Bulk("2".to_owned()),
        ]),
    );
    let harness = harness(&server, Some("ns"), 60).await;
    let cached = harness
        .cache
        .async_batch_get_cache(&[Some("alpha"), Some("alpha")])
        .await
        .expect("the batch read");
    assert_eq!(
        server.commands_named("MGET")[0].args,
        ["MGET", "ns:alpha", "ns:alpha"]
    );
    assert_eq!(cached.len(), 1);
    assert_eq!(cached.get("alpha"), Some(&Some(number(2))));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_empty_key_list_sends_nothing() {
    let server = FakeServer::start().await;
    let harness = harness(&server, Some("ns"), 60).await;
    let cached = harness
        .cache
        .async_batch_get_cache(&[None, None])
        .await
        .expect("the batch read");
    assert!(cached.is_empty());
    assert!(server.commands_named("MGET").is_empty());
    assert_eq!(
        harness.hooks.for_call("async_batch_get_cache"),
        ["success:async_batch_get_cache:None"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_client_error_is_swallowed_into_an_empty_map() {
    let harness = harness_at("127.0.0.1", 1, Some("ns"), 60).await;
    let cached = harness
        .cache
        .async_batch_get_cache(&[Some("alpha")])
        .await
        .expect("the swallow");
    assert!(cached.is_empty());
    let hooks = harness.hooks.for_call("async_batch_get_cache");
    assert_eq!(hooks.len(), 1);
    assert!(hooks[0].starts_with("failure:async_batch_get_cache:None:"));
    let gate = harness.gate.entries();
    assert_eq!(gate[0], "enter:async_batch_get_cache");
    assert!(gate[1].starts_with("swallowed:1:"));
    assert_eq!(gate[2], "exit:1");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_mget_error_is_swallowed_into_an_empty_map() {
    let server = FakeServer::start().await;
    server.script("MGET", Reply::Error("ERR mget failed".to_owned()));
    let harness = harness(&server, Some("ns"), 60).await;
    let cached = harness
        .cache
        .async_batch_get_cache(&[Some("alpha")])
        .await
        .expect("the swallow");
    assert!(cached.is_empty());
    assert_eq!(harness.hooks.for_call("async_batch_get_cache").len(), 1);
    let gate = harness.gate.entries();
    assert_eq!(gate[0], "enter:async_batch_get_cache");
    assert!(gate[1].starts_with("swallowed:1:"));
    assert_eq!(gate[2], "exit:1");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bad_cached_value_fires_both_hooks() {
    let server = FakeServer::start().await;
    server.script(
        "MGET",
        Reply::Array(vec![Reply::Bulk("not a cached value".to_owned())]),
    );
    let harness = harness(&server, Some("ns"), 60).await;
    let cached = harness
        .cache
        .async_batch_get_cache(&[Some("alpha")])
        .await
        .expect("the swallow");
    assert!(cached.is_empty());
    let hooks = harness.hooks.for_call("async_batch_get_cache");
    assert_eq!(hooks.len(), 2);
    assert_eq!(hooks[0], "success:async_batch_get_cache:None");
    assert!(hooks[1].starts_with("failure:async_batch_get_cache:None:"));
    let gate = harness.gate.entries();
    assert_eq!(gate[0], "enter:async_batch_get_cache");
    assert!(gate[1].starts_with("swallowed:1:"));
    assert_eq!(gate[2], "exit:1");
}
