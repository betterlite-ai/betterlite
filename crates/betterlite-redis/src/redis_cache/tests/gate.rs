use super::support::{TestGate, harness, harness_with_gate};
use crate::redis_cache::RedisCache;
use crate::redis_cache::error::RedisCacheError;
use crate::redis_counters::scripts::RegisteredScript;
use crate::redis_live_client::tests::fake_server::{FakeServer, Reply};

fn data_commands(server: &FakeServer) -> Vec<(String, Vec<String>)> {
    server
        .commands()
        .into_iter()
        .filter(|recorded| !matches!(recorded.name.as_str(), "PING" | "CLIENT"))
        .map(|recorded| (recorded.name, recorded.args))
        .collect()
}

async fn breaker_errors(cache: &RedisCache, script: &RegisteredScript) -> Vec<RedisCacheError> {
    vec![
        cache
            .async_increment("alpha", 1.0_f64, None, false)
            .await
            .expect_err("the breaker on async_increment"),
        cache
            .async_increment_pipeline(&[])
            .await
            .expect_err("the breaker on async_increment_pipeline"),
        cache
            .async_get_cache("alpha")
            .await
            .expect_err("the breaker on async_get_cache"),
        cache
            .async_batch_get_cache(&[Some("alpha")])
            .await
            .expect_err("the breaker on async_batch_get_cache"),
        cache
            .async_get_ttl("alpha")
            .await
            .expect_err("the breaker on async_get_ttl"),
        cache
            .async_refresh_ttl("alpha", None)
            .await
            .expect_err("the breaker on async_refresh_ttl"),
        cache
            .run_script(script, &[], &[])
            .await
            .expect_err("the breaker on run_script"),
    ]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_open_breaker_returns_the_breaker_error_from_every_method_with_nothing_sent() {
    let server = FakeServer::start().await;
    let harness = harness_with_gate(&server, Some("ns"), 60, TestGate::always()).await;
    let script = harness.cache.counters.async_register_script("return 1");
    let before = data_commands(&server);

    let errors = breaker_errors(&harness.cache, &script).await;

    assert!(
        errors
            .iter()
            .all(|error| matches!(error, RedisCacheError::Breaker(_)))
    );
    assert_eq!(data_commands(&server), before);
    assert_eq!(
        harness.gate.entries(),
        [
            "enter:async_increment",
            "enter:async_increment_pipeline",
            "enter:async_get_cache",
            "enter:async_batch_get_cache",
            "enter:async_get_ttl",
            "enter:async_refresh_ttl",
            "enter:run_script",
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_success_exits_the_gate_once() {
    let server = FakeServer::start().await;
    server.script("EXPIRE", Reply::Int(1));
    let harness = harness(&server, None, 60).await;
    harness
        .cache
        .async_refresh_ttl("alpha", None)
        .await
        .expect("the refresh");
    assert_eq!(
        harness.gate.entries(),
        ["enter:async_refresh_ttl", "exit:1"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_returned_error_skips_the_exit() {
    let server = FakeServer::start().await;
    server.script("EXPIRE", Reply::Error("ERR expire failed".to_owned()));
    let harness = harness(&server, None, 60).await;
    let _error = harness
        .cache
        .async_refresh_ttl("alpha", None)
        .await
        .expect_err("the command error");
    let gate = harness.gate.entries();
    assert_eq!(gate[0], "enter:async_refresh_ttl");
    assert!(gate[1].starts_with("failure:"));
    assert_eq!(gate.len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_swallowed_failure_reaches_the_gate_before_the_exit() {
    let server = FakeServer::start().await;
    server.script("TTL", Reply::Error("ERR ttl failed".to_owned()));
    let harness = harness(&server, None, 60).await;
    harness
        .cache
        .async_get_ttl("alpha")
        .await
        .expect("the swallow");
    let gate = harness.gate.entries();
    assert_eq!(gate[0], "enter:async_get_ttl");
    assert!(gate[1].starts_with("swallowed:1:"));
    assert_eq!(gate[2], "exit:1");
    assert_eq!(gate.len(), 3);
}
