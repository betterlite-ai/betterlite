use std::collections::BTreeMap;

use super::support::{TestGate, harness_with_gate, harness_without_server};
use crate::redis_live_client::tests::fake_server::FakeServer;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stores_the_json_text_with_the_ttl() {
    let server = FakeServer::start().await;
    let harness = harness_with_gate(&server, Some("ns"), Some(60), None, TestGate::never());
    let value = BTreeMap::from([("answer", 42_i32)]);
    harness.values.set_cache("alpha", &value, Some(5.0_f64));
    let sets = server.commands_named("SET");
    assert_eq!(sets.len(), 1);
    assert_eq!(
        sets[0].args,
        ["SET", "ns:alpha", "{\"answer\":42}", "EX", "5"]
    );
    assert_eq!(
        harness.hooks.entries(),
        vec!["success:set_cache:None".to_owned()]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sends_no_ex_without_a_ttl() {
    let server = FakeServer::start().await;
    let harness = harness_with_gate(&server, None, None, None, TestGate::never());
    harness.values.set_cache("alpha", "cached", None);
    assert_eq!(
        server.commands_named("SET")[0].args,
        ["SET", "alpha", "\"cached\""]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn never_calls_the_gate_even_when_the_breaker_is_open() {
    let server = FakeServer::start().await;
    let harness = harness_with_gate(&server, Some("ns"), Some(60), None, TestGate::always());
    harness.values.set_cache("alpha", "cached", None);
    assert_eq!(server.commands_named("SET").len(), 1);
    assert!(harness.gate.entries().is_empty());
}

#[tokio::test]
async fn swallows_an_error_with_no_failure_hook() {
    let harness = harness_without_server(Some("ns"), Some(60));
    harness.values.set_cache("alpha", "cached", None);
    assert!(harness.hooks.entries().is_empty());
    assert!(harness.gate.entries().is_empty());
}
