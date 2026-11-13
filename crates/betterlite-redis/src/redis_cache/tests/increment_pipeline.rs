use super::support::{
    DEFAULT_MAJOR_VERSION, TestGate, connect_cache, harness, harness_at, overrides,
    overrides_with_missing_credentials, unreadable_ca,
};
use crate::cache_primitives::pipeline_increment::IncrementOperation;
use crate::redis_cache::error::RedisCacheError;
use crate::redis_live_client::tests::fake_server::{FakeServer, Reply};

fn operation(key: &str, increment_value: f64, ttl: Option<i64>) -> IncrementOperation {
    IncrementOperation {
        key: key.to_owned(),
        increment_value,
        ttl,
    }
}

fn amount(command_args: &[String]) -> f64 {
    command_args[2].parse().expect("a float increment argument")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_empty_list_returns_none_without_a_command() {
    let server = FakeServer::start().await;
    let harness = harness(&server, Some("ns"), 60).await;
    let incremented = harness
        .cache
        .async_increment_pipeline(&[])
        .await
        .expect("the empty pipeline");
    assert_eq!(incremented, None);
    assert!(server.commands_named("INCRBYFLOAT").is_empty());
    assert!(server.commands_named("EXPIRE").is_empty());
    assert!(
        harness
            .hooks
            .for_call("async_increment_pipeline")
            .is_empty()
    );
    assert_eq!(
        harness.gate.entries(),
        ["enter:async_increment_pipeline", "exit:1"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_operation_is_incremented_with_its_own_ttl() {
    let server = FakeServer::start().await;
    server.script("INCRBYFLOAT", Reply::Bulk("1.5".to_owned()));
    server.script("INCRBYFLOAT", Reply::Bulk("2.0".to_owned()));
    server.script("EXPIRE", Reply::Int(1));
    let harness = harness(&server, Some("ns"), 60).await;
    let operations = [
        operation("alpha", 1.5_f64, Some(20)),
        operation("beta", 2.0_f64, None),
    ];
    let incremented = harness
        .cache
        .async_increment_pipeline(&operations)
        .await
        .expect("the pipeline");
    assert_eq!(incremented, Some(vec![1.5_f64, 2.0_f64]));

    let increments = server.commands_named("INCRBYFLOAT");
    assert_eq!(increments.len(), 2);
    assert_eq!(increments[0].args[1], "ns:alpha");
    assert_eq!(Some(amount(&increments[0].args)), Some(1.5_f64));
    assert_eq!(increments[1].args[1], "ns:beta");
    assert_eq!(Some(amount(&increments[1].args)), Some(2.0_f64));
    let expires = server.commands_named("EXPIRE");
    assert_eq!(expires.len(), 1);
    assert_eq!(expires[0].args, ["EXPIRE", "ns:alpha", "20"]);
    assert_eq!(
        harness.hooks.for_call("async_increment_pipeline"),
        ["success:async_increment_pipeline:None"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unreachable_redis_fires_the_failure_hook_and_returns_the_error() {
    let harness = harness_at("127.0.0.1", 1, Some("ns"), 60).await;
    let error = harness
        .cache
        .async_increment_pipeline(&[operation("alpha", 1.0_f64, None)])
        .await
        .expect_err("the connection error");
    assert!(matches!(error, RedisCacheError::ClientReuse(_)));
    let hooks = harness.hooks.for_call("async_increment_pipeline");
    assert_eq!(hooks.len(), 1);
    assert!(hooks[0].starts_with("failure:async_increment_pipeline:None:"));
    let gate = harness.gate.entries();
    assert_eq!(gate[0], "enter:async_increment_pipeline");
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
        .async_increment_pipeline(&[operation("alpha", 1.0_f64, None)])
        .await
        .expect_err("the client build error");
    assert!(matches!(error, RedisCacheError::ClientReuse(_)));
    assert!(
        harness
            .hooks
            .for_call("async_increment_pipeline")
            .is_empty()
    );
    let gate = harness.gate.entries();
    assert_eq!(gate[0], "enter:async_increment_pipeline");
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
        .async_increment_pipeline(&[operation("alpha", 1.0_f64, None)])
        .await
        .expect_err("the tls file error");
    assert!(matches!(error, RedisCacheError::ClientReuse(_)));
    let hooks = harness.hooks.for_call("async_increment_pipeline");
    assert_eq!(hooks.len(), 1);
    assert!(hooks[0].starts_with("failure:async_increment_pipeline:None:"));
    let gate = harness.gate.entries();
    assert_eq!(gate[0], "enter:async_increment_pipeline");
    assert!(gate[1].starts_with("failure:"));
    assert_eq!(gate.len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pipeline_failure_fires_the_failure_hook_and_returns_the_error() {
    let server = FakeServer::start().await;
    server.script(
        "INCRBYFLOAT",
        Reply::Error("ERR increment failed".to_owned()),
    );
    let harness = harness(&server, Some("ns"), 60).await;
    let error = harness
        .cache
        .async_increment_pipeline(&[operation("alpha", 1.0_f64, None)])
        .await
        .expect_err("the pipeline error");
    assert!(matches!(error, RedisCacheError::Counters(_)));
    let hooks = harness.hooks.for_call("async_increment_pipeline");
    assert_eq!(hooks.len(), 1);
    assert!(hooks[0].starts_with("failure:async_increment_pipeline:None:"));
    let gate = harness.gate.entries();
    assert_eq!(gate[0], "enter:async_increment_pipeline");
    assert!(gate[1].starts_with("failure:"));
    assert_eq!(gate.len(), 2);
}
