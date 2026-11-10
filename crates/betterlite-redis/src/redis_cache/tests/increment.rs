use super::support::{
    DEFAULT_MAJOR_VERSION, TestGate, connect_cache, harness, harness_at, overrides,
    overrides_with_missing_credentials, unreadable_ca,
};
use crate::redis_cache::error::RedisCacheError;
use crate::redis_live_client::tests::fake_server::{FakeServer, Reply};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_given_ttl_expires_the_namespaced_key() {
    let server = FakeServer::start().await;
    server.script("INCRBYFLOAT", Reply::Bulk("2.5".to_owned()));
    server.script("EXPIRE", Reply::Int(1));
    let harness = harness(&server, Some("ns"), 60).await;
    let incremented = harness
        .cache
        .async_increment("alpha", 2.5_f64, Some(30), true)
        .await
        .expect("the increment");
    assert_eq!(Some(incremented), Some(2.5_f64));
    assert_eq!(
        server.commands_named("INCRBYFLOAT")[0].args,
        ["INCRBYFLOAT", "ns:alpha", "2.5"]
    );
    assert_eq!(
        server.commands_named("EXPIRE")[0].args,
        ["EXPIRE", "ns:alpha", "30"]
    );
    assert!(server.commands_named("TTL").is_empty());
    assert_eq!(
        harness.hooks.for_call("async_increment"),
        ["success:async_increment:None"]
    );
    assert_eq!(harness.gate.entries(), ["enter:async_increment", "exit:1"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_missing_ttl_uses_the_default_and_refreshes_an_unbounded_key() {
    let server = FakeServer::start().await;
    server.script("INCRBYFLOAT", Reply::Bulk("1".to_owned()));
    server.script("TTL", Reply::Int(-1));
    server.script("EXPIRE", Reply::Int(1));
    let harness = harness(&server, Some("ns"), 60).await;
    let incremented = harness
        .cache
        .async_increment("alpha", 1.0_f64, None, false)
        .await
        .expect("the increment");
    assert_eq!(Some(incremented), Some(1.0_f64));
    assert_eq!(server.commands_named("TTL")[0].args, ["TTL", "ns:alpha"]);
    assert_eq!(
        server.commands_named("EXPIRE")[0].args,
        ["EXPIRE", "ns:alpha", "60"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refresh_ttl_false_keeps_a_bounded_key_expiry() {
    let server = FakeServer::start().await;
    server.script("INCRBYFLOAT", Reply::Bulk("1".to_owned()));
    server.script("TTL", Reply::Int(12));
    let harness = harness(&server, Some("ns"), 60).await;
    let incremented = harness
        .cache
        .async_increment("alpha", 1.0_f64, None, false)
        .await
        .expect("the increment");
    assert_eq!(Some(incremented), Some(1.0_f64));
    assert_eq!(server.commands_named("TTL")[0].args, ["TTL", "ns:alpha"]);
    assert!(server.commands_named("EXPIRE").is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unreachable_redis_fires_the_failure_hook_and_returns_the_error() {
    let harness = harness_at("127.0.0.1", 1, Some("ns"), 60).await;
    let error = harness
        .cache
        .async_increment("alpha", 1.0_f64, None, false)
        .await
        .expect_err("the connection error");
    assert!(matches!(error, RedisCacheError::ClientReuse(_)));
    let hooks = harness.hooks.for_call("async_increment");
    assert_eq!(hooks.len(), 1);
    assert!(hooks[0].starts_with("failure:async_increment:None:"));
    let gate = harness.gate.entries();
    assert_eq!(gate[0], "enter:async_increment");
    assert!(gate[1].starts_with("failure:"));
    assert_eq!(gate.len(), 2);
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
        .async_increment("alpha", 1.0_f64, None, false)
        .await
        .expect_err("the client build error");
    assert!(matches!(error, RedisCacheError::ClientReuse(_)));
    assert!(harness.hooks.for_call("async_increment").is_empty());
    let gate = harness.gate.entries();
    assert_eq!(gate[0], "enter:async_increment");
    assert!(gate[1].starts_with("failure:"));
    assert_eq!(gate.len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unreadable_tls_file_fires_the_failure_hook() {
    let server = FakeServer::start().await;
    let harness = connect_cache(
        unreadable_ca(overrides(&server.host(), server.port())),
        Some("ns"),
        60,
        DEFAULT_MAJOR_VERSION,
        TestGate::never(),
    )
    .await;
    let error = harness
        .cache
        .async_increment("alpha", 1.0_f64, None, false)
        .await
        .expect_err("the tls file error");
    assert!(matches!(error, RedisCacheError::ClientReuse(_)));
    let hooks = harness.hooks.for_call("async_increment");
    assert_eq!(hooks.len(), 1);
    assert!(hooks[0].starts_with("failure:async_increment:None:"));
    let gate = harness.gate.entries();
    assert_eq!(gate[0], "enter:async_increment");
    assert!(gate[1].starts_with("failure:"));
    assert_eq!(gate.len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_command_failure_fires_the_failure_hook_and_returns_the_error() {
    let server = FakeServer::start().await;
    server.script(
        "INCRBYFLOAT",
        Reply::Error("ERR increment failed".to_owned()),
    );
    let harness = harness(&server, Some("ns"), 60).await;
    let error = harness
        .cache
        .async_increment("alpha", 1.0_f64, None, false)
        .await
        .expect_err("the command error");
    assert!(matches!(error, RedisCacheError::Counters(_)));
    let hooks = harness.hooks.for_call("async_increment");
    assert_eq!(hooks.len(), 1);
    assert!(hooks[0].starts_with("failure:async_increment:None:"));
    let gate = harness.gate.entries();
    assert_eq!(gate[0], "enter:async_increment");
    assert!(gate[1].starts_with("failure:"));
    assert_eq!(gate.len(), 2);
}
