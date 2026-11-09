use redis::Value;

use crate::cache_primitives::protocol::RedisScalar;
use crate::redis_client_reuse::transport::RedisCacheFamily;
use crate::redis_counters::RedisCountersError;
use crate::redis_live_client::tests::fake_server::{FakeServer, Reply};

use super::support::{cluster_pair, counters, direct_settings, key_in_slot_range, transport};

const SCRIPT: &str = "return 1";
const SCRIPT_SHA1: &str = "e0e1f9fabfc9d4800c877a703b823ac0578ff8db";

#[tokio::test]
async fn registering_a_script_sends_nothing_and_hashes_the_source() {
    let server = FakeServer::start().await;
    let (counters, _hooks, _gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        Some("ns"),
    );
    let script = counters.async_register_script(SCRIPT);
    assert_eq!(script.source, SCRIPT);
    assert_eq!(script.sha1, SCRIPT_SHA1);
    assert!(server.commands().is_empty());
}

#[tokio::test]
async fn run_sends_evalsha_with_namespaced_keys_and_untouched_args() {
    let server = FakeServer::start().await;
    server.script("EVALSHA", Reply::Bulk("done".to_owned()));
    let (counters, _hooks, _gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        Some("ns"),
    );
    let script = counters.async_register_script(SCRIPT);
    let args = [
        RedisScalar::Text("first".to_owned()),
        RedisScalar::Float(1.5_f64),
        RedisScalar::Bytes(b"raw".to_vec()),
    ];
    let reply = script
        .run(&["key".to_owned()], &args)
        .await
        .expect("the script reply");
    assert_eq!(reply, Value::BulkString(b"done".to_vec()));
    assert_eq!(
        server.commands_named("EVALSHA")[0].args,
        ["EVALSHA", SCRIPT_SHA1, "1", "ns:key", "first", "1.5", "raw"]
    );
}

#[tokio::test]
async fn a_no_script_reply_loads_the_source_once_and_retries() {
    let server = FakeServer::start().await;
    server.script(
        "EVALSHA",
        Reply::Error("NOSCRIPT No matching script. Please use EVAL.".to_owned()),
    );
    server.script("EVALSHA", Reply::Bulk("done".to_owned()));
    server.script("SCRIPT", Reply::Bulk(SCRIPT_SHA1.to_owned()));
    let (counters, _hooks, _gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        None,
    );
    let script = counters.async_register_script(SCRIPT);
    let reply = script.run(&[], &[]).await.expect("the retried reply");
    assert_eq!(reply, Value::BulkString(b"done".to_vec()));
    assert_eq!(server.commands_named("EVALSHA").len(), 2);
    let loads = server.commands_named("SCRIPT");
    assert_eq!(loads.len(), 1);
    assert_eq!(loads[0].args, ["SCRIPT", "LOAD", SCRIPT]);
}

#[tokio::test]
async fn a_second_no_script_reply_is_returned() {
    let server = FakeServer::start().await;
    server.script(
        "EVALSHA",
        Reply::Error("NOSCRIPT No matching script. Please use EVAL.".to_owned()),
    );
    server.script(
        "EVALSHA",
        Reply::Error("NOSCRIPT No matching script. Please use EVAL.".to_owned()),
    );
    server.script("SCRIPT", Reply::Bulk(SCRIPT_SHA1.to_owned()));
    let (counters, _hooks, _gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        None,
    );
    let script = counters.async_register_script(SCRIPT);
    let error = script
        .run(&[], &[])
        .await
        .expect_err("the second no-script");
    assert!(matches!(error, RedisCountersError::Client(_)), "{error}");
    assert_eq!(server.commands_named("EVALSHA").len(), 2);
    assert_eq!(server.commands_named("SCRIPT").len(), 1);
}

#[tokio::test]
async fn another_error_is_returned_without_a_script_load() {
    let server = FakeServer::start().await;
    server.script("EVALSHA", Reply::Error("ERR script failed".to_owned()));
    let (counters, _hooks, _gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        None,
    );
    let script = counters.async_register_script(SCRIPT);
    let error = script.run(&[], &[]).await.expect_err("the script error");
    assert!(error.to_string().contains("script failed"), "{error}");
    assert!(server.commands_named("SCRIPT").is_empty());
}

#[tokio::test]
async fn a_cluster_routes_the_evalsha_to_the_node_owning_the_first_key() {
    let pair = cluster_pair().await;
    let key = key_in_slot_range(0, 8191);
    pair.first
        .script("EVALSHA", Reply::Bulk("owner".to_owned()));
    let (counters, _hooks, _gate) = counters(pair.transport, None);
    let script = counters.async_register_script(SCRIPT);
    let reply = script
        .run(&[key.to_owned()], &[])
        .await
        .expect("the script reply");
    assert_eq!(reply, Value::BulkString(b"owner".to_vec()));
    assert_eq!(pair.first.commands_named("EVALSHA").len(), 1);
    assert!(pair.second.commands_named("EVALSHA").is_empty());
}
