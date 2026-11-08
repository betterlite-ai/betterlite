use std::collections::BTreeMap;
use std::collections::HashMap;

use super::support::harness;
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
    let harness = harness(&server, Some("ns"), Some(60), None);
    let cached = harness
        .values
        .batch_get_cache(&[Some("alpha"), Some("missing")])
        .expect("the batch read");
    let expected = HashMap::from([
        ("alpha".to_owned(), Some(object())),
        ("missing".to_owned(), None),
    ]);
    assert_eq!(cached, expected);
    let mgets = server.commands_named("MGET");
    assert_eq!(mgets.len(), 1);
    assert_eq!(mgets[0].args, ["MGET", "ns:alpha", "ns:missing"]);
    assert_eq!(
        harness.hooks.entries(),
        vec!["success:batch_get_cache:None".to_owned()]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn drops_none_keys() {
    let server = FakeServer::start().await;
    server.script(
        "MGET",
        Reply::Array(vec![Reply::Nil, Reply::Bulk("2".to_owned())]),
    );
    let harness = harness(&server, Some("ns"), Some(60), None);
    let cached = harness
        .values
        .batch_get_cache(&[Some("alpha"), None, Some("beta")])
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
async fn a_duplicate_key_keeps_the_last_value() {
    let server = FakeServer::start().await;
    server.script(
        "MGET",
        Reply::Array(vec![
            Reply::Bulk("1".to_owned()),
            Reply::Bulk("2".to_owned()),
        ]),
    );
    let harness = harness(&server, None, None, None);
    let cached = harness
        .values
        .batch_get_cache(&[Some("alpha"), Some("alpha")])
        .expect("the batch read");
    assert_eq!(
        cached,
        HashMap::from([("alpha".to_owned(), Some(number(2)))])
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exits_the_gate_before_decoding_a_bad_reply() {
    let server = FakeServer::start().await;
    server.script(
        "MGET",
        Reply::Array(vec![Reply::Bulk("not a cached value".to_owned())]),
    );
    let harness = harness(&server, None, None, None);
    let cached = harness
        .values
        .batch_get_cache(&[Some("alpha")])
        .expect("the batch read");
    assert!(cached.is_empty());
    let gate = harness.gate.entries();
    assert_eq!(gate[0], "enter:batch_get_cache");
    assert_eq!(gate[1], "exit:1");
    assert!(gate[2].starts_with("swallowed:1:"));
    let hooks = harness.hooks.entries();
    assert_eq!(hooks.len(), 2);
    assert!(hooks[0].starts_with("success:batch_get_cache:"));
    assert!(hooks[1].starts_with("failure:batch_get_cache:"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_read_error_gives_an_empty_map() {
    let server = FakeServer::start().await;
    server.script("MGET", Reply::Error("ERR broken".to_owned()));
    let harness = harness(&server, None, None, None);
    let cached = harness
        .values
        .batch_get_cache(&[Some("alpha"), Some("beta")])
        .expect("the batch read");
    assert!(cached.is_empty());
    let gate = harness.gate.entries();
    assert_eq!(gate[0], "enter:batch_get_cache");
    assert!(gate[1].starts_with("swallowed:1:"));
    assert_eq!(gate.len(), 2);
    let hooks = harness.hooks.entries();
    assert_eq!(hooks.len(), 1);
    assert!(hooks[0].starts_with("failure:batch_get_cache:"));
}
