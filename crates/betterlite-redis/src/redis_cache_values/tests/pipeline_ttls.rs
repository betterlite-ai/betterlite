use std::collections::HashMap;

use super::support::{harness, harness_without_server, settings};
use crate::redis_cache_values::error::RedisCacheValuesError;
use crate::redis_live_client::tests::fake_server::{FakeServer, Reply};

fn mixed_entries() -> Vec<(String, &'static str, Option<f64>)> {
    vec![
        ("alpha".to_owned(), "first", Some(5.0_f64)),
        ("beta".to_owned(), "second", None),
    ]
}

#[tokio::test]
async fn gives_each_entry_its_own_ttl_and_uses_the_default_for_none() {
    let server = FakeServer::start().await;
    let harness = harness(&server, settings(Some("ns"), Some(60), None));
    harness
        .values
        .async_set_cache_pipeline_with_ttls(&mixed_entries())
        .await
        .expect("the pipelined write");
    let sets = server.commands_named("SET");
    assert_eq!(sets.len(), 2);
    assert_eq!(sets[0].args, ["SET", "ns:alpha", "\"first\"", "EX", "5"]);
    assert_eq!(sets[1].args, ["SET", "ns:beta", "\"second\"", "EX", "60"]);
    assert_eq!(
        harness.hooks.entries(),
        vec!["success:async_set_cache_pipeline_with_ttls:None".to_owned()]
    );
}

#[tokio::test]
async fn sends_no_ex_when_neither_ttl_exists() {
    let server = FakeServer::start().await;
    let harness = harness(&server, settings(Some("ns"), None, None));
    let entries: Vec<(String, &str, Option<f64>)> = vec![
        ("alpha".to_owned(), "first", None),
        ("beta".to_owned(), "second", None),
    ];
    harness
        .values
        .async_set_cache_pipeline_with_ttls(&entries)
        .await
        .expect("the pipelined write");
    let sets = server.commands_named("SET");
    assert_eq!(sets[0].args, ["SET", "ns:alpha", "\"first\""]);
    assert_eq!(sets[1].args, ["SET", "ns:beta", "\"second\""]);
}

#[tokio::test]
async fn an_empty_list_sends_nothing() {
    let server = FakeServer::start().await;
    let harness = harness(&server, settings(Some("ns"), Some(60), None));
    let empty: Vec<(String, &str, Option<f64>)> = Vec::new();
    harness
        .values
        .async_set_cache_pipeline_with_ttls(&empty)
        .await
        .expect("the empty pipeline");
    assert!(server.commands().is_empty());
    assert_eq!(server.connections(), 0);
}

#[tokio::test]
async fn swallows_a_client_failure() {
    let harness = harness_without_server(settings(Some("ns"), Some(60), None));
    harness
        .values
        .async_set_cache_pipeline_with_ttls(&mixed_entries())
        .await
        .expect("the swallowed client failure");
    let hooks = harness.hooks.entries();
    assert_eq!(hooks.len(), 1);
    assert!(hooks[0].starts_with("failure:async_set_cache_pipeline_with_ttls:"));
    let gate = harness.gate.entries();
    assert_eq!(gate[0], "enter:async_set_cache_pipeline_with_ttls");
    assert!(gate[1].starts_with("swallowed:1:"));
    assert_eq!(gate[2], "exit:1");
}

#[tokio::test]
async fn swallows_an_execute_failure() {
    let server = FakeServer::start().await;
    server.script("SET", Reply::Error("ERR broken".to_owned()));
    let harness = harness(&server, settings(Some("ns"), Some(60), None));
    harness
        .values
        .async_set_cache_pipeline_with_ttls(&mixed_entries())
        .await
        .expect("the swallowed execute failure");
    assert!(
        harness
            .hooks
            .entries()
            .iter()
            .any(|entry| entry.starts_with("failure:async_set_cache_pipeline_with_ttls:"))
    );
    assert!(
        harness
            .gate
            .entries()
            .iter()
            .any(|entry| entry.starts_with("swallowed:1:"))
    );
}

#[tokio::test]
async fn an_out_of_range_ttl_is_swallowed() {
    let server = FakeServer::start().await;
    let harness = harness(&server, settings(Some("ns"), None, None));
    let entries = [("alpha".to_owned(), "first", Some(f64::MAX))];
    harness
        .values
        .async_set_cache_pipeline_with_ttls(&entries)
        .await
        .expect("the swallowed TTL overflow");
    assert!(server.commands().is_empty());
    let hooks = harness.hooks.entries();
    assert_eq!(hooks.len(), 1);
    assert!(hooks[0].starts_with("failure:async_set_cache_pipeline_with_ttls:None:"));
    let gate = harness.gate.entries();
    assert_eq!(gate[0], "enter:async_set_cache_pipeline_with_ttls");
    assert!(gate[1].starts_with("swallowed:1:"));
    assert_eq!(gate[2], "exit:1");
}

#[tokio::test]
async fn an_infinite_ttl_is_returned() {
    let server = FakeServer::start().await;
    let harness = harness(&server, settings(Some("ns"), None, None));
    let entries = [("alpha".to_owned(), "first", Some(f64::INFINITY))];
    let error = harness
        .values
        .async_set_cache_pipeline_with_ttls(&entries)
        .await
        .expect_err("int() raises for an infinite TTL");
    assert!(matches!(error, RedisCacheValuesError::TtlOutOfRange { .. }));
    assert!(server.commands().is_empty());
    assert!(harness.hooks.entries().is_empty());
    let gate = harness.gate.entries();
    assert_eq!(gate[0], "enter:async_set_cache_pipeline_with_ttls");
    assert!(gate[1].starts_with("failure:"));
    assert_eq!(gate.len(), 2);
}

#[tokio::test]
async fn an_unserializable_value_is_returned() {
    let server = FakeServer::start().await;
    let harness = harness(&server, settings(Some("ns"), None, None));
    let value = HashMap::from([((1_u8, 2_u8), "value")]);
    let entries = vec![("alpha".to_owned(), value, None)];
    let error = harness
        .values
        .async_set_cache_pipeline_with_ttls(&entries)
        .await
        .expect_err("a tuple key cannot be a JSON object key");
    assert!(matches!(error, RedisCacheValuesError::Serialize(_)));
    assert!(server.commands().is_empty());
    assert!(harness.hooks.entries().is_empty());
    let gate = harness.gate.entries();
    assert_eq!(gate[0], "enter:async_set_cache_pipeline_with_ttls");
    assert!(gate[1].starts_with("failure:"));
    assert_eq!(gate.len(), 2);
}
