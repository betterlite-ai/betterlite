use crate::cache_primitives::decode::INCREMENT_WITH_FLOOR_LUA;
use crate::redis_client_reuse::transport::RedisCacheFamily;
use crate::redis_counters::RedisCountersError;
use crate::redis_live_client::tests::fake_server::{FakeServer, Reply};

use super::support::{counters, direct_settings, runtime, transport};

#[test]
fn increment_with_floor_sends_the_floor_script_with_text_arguments() {
    let runtime = runtime();
    let server = runtime.block_on(FakeServer::start());
    server.script("EVAL", Reply::Int(4));
    let (counters, _hooks, gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        None,
    );
    let count = counters
        .increment_with_floor("limit", 2, 30)
        .expect("the count");
    assert_eq!(count, 4);
    let eval = server.commands_named("EVAL");
    assert_eq!(eval.len(), 1);
    assert_eq!(
        eval[0].args,
        ["EVAL", INCREMENT_WITH_FLOOR_LUA, "1", "limit", "2", "30"]
    );
    assert_eq!(gate.enters(), vec!["increment_with_floor"]);
}

#[test]
fn a_bulk_string_floor_reply_is_the_count() {
    let runtime = runtime();
    let server = runtime.block_on(FakeServer::start());
    server.script("EVAL", Reply::Bulk("7".to_owned()));
    let (counters, _hooks, _gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        Some("ns"),
    );
    let count = counters
        .increment_with_floor("limit", 1, 45)
        .expect("the count");
    assert_eq!(count, 7);
    assert_eq!(
        server.commands_named("EVAL")[0].args,
        ["EVAL", INCREMENT_WITH_FLOOR_LUA, "1", "ns:limit", "1", "45"]
    );
}

#[test]
fn a_nil_floor_reply_is_unexpected() {
    let runtime = runtime();
    let server = runtime.block_on(FakeServer::start());
    server.script("EVAL", Reply::Nil);
    let (counters, _hooks, _gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        None,
    );
    let error = counters
        .increment_with_floor("limit", 1, 30)
        .expect_err("the nil reply");
    assert!(matches!(
        error,
        RedisCountersError::UnexpectedReply {
            operation: "increment_with_floor"
        }
    ));
}

#[test]
fn an_array_floor_reply_is_unexpected() {
    let runtime = runtime();
    let server = runtime.block_on(FakeServer::start());
    server.script("EVAL", Reply::Array(vec![]));
    let (counters, _hooks, _gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        None,
    );
    let error = counters
        .increment_with_floor("limit", 1, 30)
        .expect_err("the array reply");
    assert!(matches!(
        error,
        RedisCountersError::UnexpectedReply {
            operation: "increment_with_floor"
        }
    ));
}

#[test]
fn a_server_error_comes_back_unchanged() {
    let runtime = runtime();
    let server = runtime.block_on(FakeServer::start());
    server.script("EVAL", Reply::Error("ERR floor script failed".to_owned()));
    let (counters, _hooks, gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        None,
    );
    let error = counters
        .increment_with_floor("limit", 1, 30)
        .expect_err("the server error");
    assert!(error.to_string().contains("floor script failed"), "{error}");
    assert_eq!(gate.failures().len(), 1);
    assert!(gate.exits().is_empty());
}

#[tokio::test]
async fn async_increment_with_floor_sends_the_floor_script() {
    let server = FakeServer::start().await;
    server.script("EVAL", Reply::Bulk("9".to_owned()));
    let (counters, _hooks, gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        Some("ns"),
    );
    let count = counters
        .async_increment_with_floor("limit", 3, 45)
        .await
        .expect("the count");
    assert_eq!(count, 9);
    let eval = server.commands_named("EVAL");
    assert_eq!(eval.len(), 1);
    assert_eq!(
        eval[0].args,
        ["EVAL", INCREMENT_WITH_FLOOR_LUA, "1", "ns:limit", "3", "45"]
    );
    assert_eq!(gate.enters(), vec!["async_increment_with_floor"]);
}

#[tokio::test]
async fn an_integer_async_floor_reply_is_the_count() {
    let server = FakeServer::start().await;
    server.script("EVAL", Reply::Int(11));
    let (counters, _hooks, _gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        None,
    );
    let count = counters
        .async_increment_with_floor("limit", 3, 45)
        .await
        .expect("the count");
    assert_eq!(count, 11);
}

#[tokio::test]
async fn a_non_numeric_async_floor_reply_is_unexpected() {
    let server = FakeServer::start().await;
    server.script("EVAL", Reply::Nil);
    let (counters, _hooks, _gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        None,
    );
    let error = counters
        .async_increment_with_floor("limit", 3, 45)
        .await
        .expect_err("the nil reply");
    assert!(matches!(
        error,
        RedisCountersError::UnexpectedReply {
            operation: "async_increment_with_floor"
        }
    ));
}

#[tokio::test]
async fn an_async_server_error_comes_back_unchanged() {
    let server = FakeServer::start().await;
    server.script("EVAL", Reply::Error("ERR floor script failed".to_owned()));
    let (counters, _hooks, _gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        None,
    );
    let error = counters
        .async_increment_with_floor("limit", 1, 30)
        .await
        .expect_err("the server error");
    assert!(error.to_string().contains("floor script failed"), "{error}");
}
