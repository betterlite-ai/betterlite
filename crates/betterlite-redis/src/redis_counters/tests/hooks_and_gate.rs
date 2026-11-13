use secrecy::SecretString;

use crate::redis_client_reuse::transport::RedisCacheFamily;
use crate::redis_counters::RedisCountersError;
use crate::redis_live_client::tests::fake_server::{FakeServer, Reply};

use super::support::{ADMISSION, capture_logs, counters, direct_settings, runtime, transport};

#[test]
fn increment_cache_fires_the_three_timed_success_hooks() {
    let runtime = runtime();
    let server = runtime.block_on(FakeServer::start());
    server.script("INCRBY", Reply::Int(5));
    server.script("TTL", Reply::Int(-1));
    let (counters, hooks, _gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        None,
    );
    counters
        .increment_cache("counter", 1, None)
        .expect("the count");
    assert_eq!(
        hooks.success_names(),
        vec![
            "increment_cache",
            "increment_cache_ttl",
            "increment_cache_expire"
        ]
    );
    assert!(hooks.failures().is_empty());
}

#[test]
fn increment_cache_fires_a_failure_hook_named_increment_cache() {
    let runtime = runtime();
    let server = runtime.block_on(FakeServer::start());
    server.script("INCRBY", Reply::Error("ERR increment failed".to_owned()));
    let (counters, hooks, _gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        None,
    );
    let error = counters
        .increment_cache("counter", 1, None)
        .expect_err("the increment error");
    assert!(error.to_string().contains("increment failed"), "{error}");
    assert_eq!(hooks.failure_names(), vec!["increment_cache"]);
    assert!(hooks.successes().is_empty());
}

#[test]
fn increment_cache_logs_the_operation_and_error_without_the_password() {
    let runtime = runtime();
    let server = runtime.block_on(FakeServer::start());
    server.script("INCRBY", Reply::Error("ERR increment failed".to_owned()));
    let mut settings = direct_settings(&server);
    settings.password = Some(SecretString::from("top-secret-password"));
    let (counters, hooks, _gate) =
        counters(transport(settings, RedisCacheFamily::Standalone), None);
    let (log, guard) = capture_logs();
    let error = counters
        .increment_cache("counter", 1, None)
        .expect_err("the increment error");
    drop(guard);
    let text = log.text();
    assert!(text.contains("increment_cache"), "{text}");
    assert!(text.contains("increment failed"), "{text}");
    assert!(!text.contains("top-secret-password"), "{text}");
    assert!(!error.to_string().contains("top-secret-password"));
    let failures = hooks.failures();
    assert_eq!(failures.len(), 1);
    assert!(
        !failures[0]
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("top-secret-password")
    );
}

#[tokio::test]
async fn async_scan_iter_fires_one_success_hook() {
    let server = FakeServer::start().await;
    server.script(
        "SCAN",
        Reply::Array(vec![
            Reply::Bulk("0".to_owned()),
            Reply::Array(vec![Reply::Bulk("one".to_owned())]),
        ]),
    );
    let (counters, hooks, _gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        None,
    );
    counters
        .async_scan_iter("prefix", 5)
        .await
        .expect("the keys");
    assert_eq!(hooks.success_names(), vec!["async_scan_iter"]);
    assert!(hooks.failures().is_empty());
}

#[tokio::test]
async fn async_scan_iter_fires_one_failure_hook() {
    let server = FakeServer::start().await;
    server.script("SCAN", Reply::Error("ERR scan failed".to_owned()));
    let (counters, hooks, _gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        None,
    );
    let error = counters
        .async_scan_iter("prefix", 5)
        .await
        .expect_err("the scan error");
    assert!(error.to_string().contains("scan failed"), "{error}");
    assert_eq!(hooks.failure_names(), vec!["async_scan_iter"]);
    assert!(hooks.successes().is_empty());
}

#[test]
fn increment_with_floor_enters_and_exits_the_gate_once() {
    let runtime = runtime();
    let server = runtime.block_on(FakeServer::start());
    server.script("EVAL", Reply::Int(1));
    let (counters, _hooks, gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        None,
    );
    counters
        .increment_with_floor("limit", 1, 30)
        .expect("the count");
    assert_eq!(gate.enters(), vec!["increment_with_floor"]);
    assert_eq!(gate.exits(), vec![ADMISSION]);
    assert!(gate.failures().is_empty());
}

#[test]
fn batch_get_counts_enters_and_exits_the_gate_once() {
    let runtime = runtime();
    let server = runtime.block_on(FakeServer::start());
    server.script("MGET", Reply::Array(vec![Reply::Bulk("1".to_owned())]));
    let (counters, _hooks, gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        None,
    );
    counters
        .batch_get_counts(&["alpha".to_owned()])
        .expect("the counts");
    assert_eq!(gate.enters(), vec!["batch_get_counts"]);
    assert_eq!(gate.exits(), vec![ADMISSION]);
}

#[tokio::test]
async fn async_increment_with_floor_enters_and_exits_the_gate_once() {
    let server = FakeServer::start().await;
    server.script("EVAL", Reply::Int(1));
    let (counters, _hooks, gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        None,
    );
    counters
        .async_increment_with_floor("limit", 1, 30)
        .await
        .expect("the count");
    assert_eq!(gate.enters(), vec!["async_increment_with_floor"]);
    assert_eq!(gate.exits(), vec![ADMISSION]);
}

#[tokio::test]
async fn async_batch_get_counts_enters_and_exits_the_gate_once() {
    let server = FakeServer::start().await;
    server.script("MGET", Reply::Array(vec![Reply::Bulk("1".to_owned())]));
    let (counters, _hooks, gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        None,
    );
    counters
        .async_batch_get_counts(&["alpha".to_owned()])
        .await
        .expect("the counts");
    assert_eq!(gate.enters(), vec!["async_batch_get_counts"]);
    assert_eq!(gate.exits(), vec![ADMISSION]);
}

#[tokio::test]
async fn async_scan_iter_enters_and_exits_the_gate_once() {
    let server = FakeServer::start().await;
    server.script(
        "SCAN",
        Reply::Array(vec![Reply::Bulk("0".to_owned()), Reply::Array(vec![])]),
    );
    let (counters, _hooks, gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        None,
    );
    counters
        .async_scan_iter("prefix", 5)
        .await
        .expect("the keys");
    assert_eq!(gate.enters(), vec!["async_scan_iter"]);
    assert_eq!(gate.exits(), vec![ADMISSION]);
}

#[tokio::test]
async fn async_set_max_enters_and_exits_the_gate_once() {
    let server = FakeServer::start().await;
    server.script("EVAL", Reply::Bulk("1".to_owned()));
    let (counters, _hooks, gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        None,
    );
    counters
        .async_set_max("limit", 1.0_f64, Some(30))
        .await
        .expect("the max");
    assert_eq!(gate.enters(), vec!["async_set_max"]);
    assert_eq!(gate.exits(), vec![ADMISSION]);
}

#[test]
fn an_open_breaker_stops_increment_with_floor_with_nothing_sent() {
    let runtime = runtime();
    let server = runtime.block_on(FakeServer::start());
    let (counters, _hooks, gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        None,
    );
    gate.set_open(true);
    let error = counters
        .increment_with_floor("limit", 1, 30)
        .expect_err("the open breaker");
    assert!(matches!(
        error,
        RedisCountersError::Breaker(open) if open.name == "increment_with_floor"
    ));
    assert!(server.commands().is_empty());
    assert!(gate.enters().is_empty());
}

#[test]
fn an_open_breaker_stops_batch_get_counts_with_nothing_sent() {
    let runtime = runtime();
    let server = runtime.block_on(FakeServer::start());
    let (counters, _hooks, gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        None,
    );
    gate.set_open(true);
    let error = counters
        .batch_get_counts(&["alpha".to_owned()])
        .expect_err("the open breaker");
    assert!(matches!(
        error,
        RedisCountersError::Breaker(open) if open.name == "batch_get_counts"
    ));
    assert!(server.commands().is_empty());
}

#[tokio::test]
async fn an_open_breaker_stops_the_async_guarded_methods_with_nothing_sent() {
    let server = FakeServer::start().await;
    let (counters, _hooks, gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        None,
    );
    gate.set_open(true);
    let floor = counters
        .async_increment_with_floor("limit", 1, 30)
        .await
        .expect_err("the open breaker");
    assert!(matches!(
        floor,
        RedisCountersError::Breaker(open) if open.name == "async_increment_with_floor"
    ));
    let counts = counters
        .async_batch_get_counts(&["alpha".to_owned()])
        .await
        .expect_err("the open breaker");
    assert!(matches!(
        counts,
        RedisCountersError::Breaker(open) if open.name == "async_batch_get_counts"
    ));
    let scan = counters
        .async_scan_iter("prefix", 5)
        .await
        .expect_err("the open breaker");
    assert!(matches!(
        scan,
        RedisCountersError::Breaker(open) if open.name == "async_scan_iter"
    ));
    let max = counters
        .async_set_max("limit", 1.0_f64, Some(30))
        .await
        .expect_err("the open breaker");
    assert!(matches!(
        max,
        RedisCountersError::Breaker(open) if open.name == "async_set_max"
    ));
    assert!(server.commands().is_empty());
}
