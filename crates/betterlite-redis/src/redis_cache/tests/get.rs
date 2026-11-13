use std::collections::BTreeMap;
use std::time::Duration;

use super::support::{
    DEFAULT_MAJOR_VERSION, TestGate, connect_cache, harness, harness_at, overrides,
    overrides_with_missing_credentials, unreadable_ca,
};
use crate::cache_primitives::decode::CachedValue;
use crate::redis_cache::error::RedisCacheError;
use crate::redis_live_client::tests::fake_server::{FakeServer, Reply};

fn answer() -> CachedValue {
    CachedValue::Object(BTreeMap::from([(
        "answer".to_owned(),
        CachedValue::Number(serde_json::Number::from(42)),
    )]))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn decodes_a_json_reply() {
    let server = FakeServer::start().await;
    server.script("GET", Reply::Bulk("{\"answer\": 42}".to_owned()));
    let harness = harness(&server, Some("ns"), 60).await;
    let cached = harness
        .cache
        .async_get_cache("alpha")
        .await
        .expect("the read");
    assert_eq!(cached, Some(answer()));
    assert_eq!(server.commands_named("GET")[0].args, ["GET", "ns:alpha"]);
    assert_eq!(
        harness.hooks.for_call("async_get_cache"),
        ["success:async_get_cache:Some(\"ns:alpha\")"]
    );
    let durations = harness.hooks.durations_for("async_get_cache");
    assert_eq!(durations.len(), 1);
    assert!(durations[0] > Duration::ZERO);
    assert_eq!(harness.gate.entries(), ["enter:async_get_cache", "exit:1"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn decodes_a_python_literal_reply() {
    let server = FakeServer::start().await;
    server.script("GET", Reply::Bulk("{'answer': 42}".to_owned()));
    let harness = harness(&server, Some("ns"), 60).await;
    let cached = harness
        .cache
        .async_get_cache("alpha")
        .await
        .expect("the read");
    assert_eq!(cached, Some(answer()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_miss_returns_none() {
    let server = FakeServer::start().await;
    server.script("GET", Reply::Nil);
    let harness = harness(&server, Some("ns"), 60).await;
    let cached = harness
        .cache
        .async_get_cache("alpha")
        .await
        .expect("the read");
    assert_eq!(cached, None);
    assert_eq!(
        harness.hooks.for_call("async_get_cache"),
        ["success:async_get_cache:Some(\"ns:alpha\")"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unreachable_redis_is_swallowed_into_none() {
    let harness = harness_at("127.0.0.1", 1, Some("ns"), 60).await;
    let cached = harness
        .cache
        .async_get_cache("alpha")
        .await
        .expect("the swallow");
    assert_eq!(cached, None);
    let hooks = harness.hooks.for_call("async_get_cache");
    assert_eq!(hooks.len(), 1);
    assert!(hooks[0].starts_with("failure:async_get_cache:Some(\"ns:alpha\"):"));
    let gate = harness.gate.entries();
    assert_eq!(gate[0], "enter:async_get_cache");
    assert!(gate[1].starts_with("swallowed:1:"));
    assert_eq!(gate[2], "exit:1");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_client_build_error_returns_before_any_hook() {
    let server = FakeServer::start().await;
    let harness = connect_cache(
        overrides_with_missing_credentials(&server.host(), server.port()),
        Some("ns"),
        60,
        DEFAULT_MAJOR_VERSION,
        TestGate::never(),
    )
    .await;
    let error = harness
        .cache
        .async_get_cache("alpha")
        .await
        .expect_err("the client build error");
    assert!(matches!(error, RedisCacheError::ClientReuse(_)));
    assert!(harness.hooks.for_call("async_get_cache").is_empty());
    let gate = harness.gate.entries();
    assert_eq!(gate[0], "enter:async_get_cache");
    assert!(gate[1].starts_with("failure:"));
    assert_eq!(gate.len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unreadable_tls_file_is_swallowed_into_none() {
    let server = FakeServer::start().await;
    let harness = connect_cache(
        unreadable_ca(overrides(&server.host(), server.port())),
        Some("ns"),
        60,
        DEFAULT_MAJOR_VERSION,
        TestGate::never(),
    )
    .await;
    let cached = harness
        .cache
        .async_get_cache("alpha")
        .await
        .expect("the swallow");
    assert_eq!(cached, None);
    let hooks = harness.hooks.for_call("async_get_cache");
    assert_eq!(hooks.len(), 1);
    assert!(hooks[0].starts_with("failure:async_get_cache:Some(\"ns:alpha\"):"));
    let gate = harness.gate.entries();
    assert_eq!(gate[0], "enter:async_get_cache");
    assert!(gate[1].starts_with("swallowed:1:"));
    assert_eq!(gate[2], "exit:1");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_command_error_is_swallowed() {
    let server = FakeServer::start().await;
    server.script("GET", Reply::Error("ERR get failed".to_owned()));
    let harness = harness(&server, Some("ns"), 60).await;
    let cached = harness
        .cache
        .async_get_cache("alpha")
        .await
        .expect("the swallow");
    assert_eq!(cached, None);
    let hooks = harness.hooks.for_call("async_get_cache");
    assert_eq!(hooks.len(), 1);
    assert!(hooks[0].starts_with("failure:async_get_cache:Some(\"ns:alpha\"):"));
    let gate = harness.gate.entries();
    assert_eq!(gate[0], "enter:async_get_cache");
    assert!(gate[1].starts_with("swallowed:1:"));
    assert_eq!(gate[2], "exit:1");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_undecodable_reply_is_swallowed() {
    let server = FakeServer::start().await;
    server.script("GET", Reply::Bulk("not a cached value".to_owned()));
    let harness = harness(&server, Some("ns"), 60).await;
    let cached = harness
        .cache
        .async_get_cache("alpha")
        .await
        .expect("the swallow");
    assert_eq!(cached, None);
    let hooks = harness.hooks.for_call("async_get_cache");
    assert_eq!(hooks.len(), 1);
    assert!(hooks[0].starts_with("failure:async_get_cache:Some(\"ns:alpha\"):"));
    let gate = harness.gate.entries();
    assert_eq!(gate[0], "enter:async_get_cache");
    assert!(gate[1].starts_with("swallowed:1:"));
    assert_eq!(gate[2], "exit:1");
}
