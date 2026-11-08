use std::collections::BTreeMap;

use super::support::harness;
use crate::cache_primitives::decode::CachedValue;
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
    let harness = harness(&server, Some("ns"), Some(60), None);
    let cached = harness.values.get_cache("alpha").expect("the read");
    assert_eq!(cached, Some(answer()));
    assert_eq!(server.commands_named("GET")[0].args, ["GET", "ns:alpha"]);
    assert_eq!(
        harness.hooks.entries(),
        vec!["success:get_cache:None".to_owned()]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn decodes_a_python_literal_reply() {
    let server = FakeServer::start().await;
    server.script("GET", Reply::Bulk("{'answer': 42}".to_owned()));
    let harness = harness(&server, Some("ns"), Some(60), None);
    let cached = harness.values.get_cache("alpha").expect("the read");
    assert_eq!(cached, Some(answer()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_miss_returns_none() {
    let server = FakeServer::start().await;
    server.script("GET", Reply::Nil);
    let harness = harness(&server, Some("ns"), Some(60), None);
    let cached = harness.values.get_cache("alpha").expect("the read");
    assert_eq!(cached, None);
    assert_eq!(
        harness.hooks.entries(),
        vec!["success:get_cache:None".to_owned()]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_undecodable_reply_is_swallowed() {
    let server = FakeServer::start().await;
    server.script("GET", Reply::Bulk("not a cached value".to_owned()));
    let harness = harness(&server, Some("ns"), Some(60), None);
    let cached = harness.values.get_cache("alpha").expect("the read");
    assert_eq!(cached, None);
    let gate = harness.gate.entries();
    assert_eq!(gate[0], "enter:get_cache");
    assert!(gate[1].starts_with("swallowed:1:"));
    assert_eq!(gate[2], "exit:1");
    assert_eq!(
        harness.hooks.entries(),
        vec!["success:get_cache:None".to_owned()]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_read_error_is_swallowed() {
    let server = FakeServer::start().await;
    server.script("GET", Reply::Error("ERR broken".to_owned()));
    let harness = harness(&server, Some("ns"), Some(60), None);
    let cached = harness.values.get_cache("alpha").expect("the read");
    assert_eq!(cached, None);
    let gate = harness.gate.entries();
    assert_eq!(gate[0], "enter:get_cache");
    assert!(gate[1].starts_with("swallowed:1:"));
    assert_eq!(gate[2], "exit:1");
    assert!(harness.hooks.entries().is_empty());
}
