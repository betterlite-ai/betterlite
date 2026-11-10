use rstest::rstest;

use super::support::{harness, harness_at};
use crate::redis_cache::error::RedisCacheError;
use crate::redis_live_client::tests::fake_server::{FakeServer, Reply};

#[rstest]
#[case::applied(1, true)]
#[case::key_absent(0, false)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_expire_reply_becomes_the_answer(#[case] reply: i64, #[case] expected: bool) {
    let server = FakeServer::start().await;
    server.script("EXPIRE", Reply::Int(reply));
    let harness = harness(&server, Some("ns"), 60).await;
    let refreshed = harness
        .cache
        .async_refresh_ttl("alpha", Some(30))
        .await
        .expect("the refresh");
    assert_eq!(refreshed, expected);
    assert_eq!(
        server.commands_named("EXPIRE")[0].args,
        ["EXPIRE", "ns:alpha", "30"]
    );
    assert!(harness.hooks.for_call("async_refresh_ttl").is_empty());
    assert_eq!(
        harness.gate.entries(),
        ["enter:async_refresh_ttl", "exit:1"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_missing_ttl_uses_the_default() {
    let server = FakeServer::start().await;
    server.script("EXPIRE", Reply::Int(1));
    let harness = harness(&server, Some("ns"), 60).await;
    let refreshed = harness
        .cache
        .async_refresh_ttl("alpha", None)
        .await
        .expect("the refresh");
    assert!(refreshed);
    assert_eq!(
        server.commands_named("EXPIRE")[0].args,
        ["EXPIRE", "ns:alpha", "60"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_error_is_returned_and_recorded() {
    let server = FakeServer::start().await;
    server.script("EXPIRE", Reply::Error("ERR expire failed".to_owned()));
    let harness = harness(&server, Some("ns"), 60).await;
    let error = harness
        .cache
        .async_refresh_ttl("alpha", None)
        .await
        .expect_err("the command error");
    assert!(matches!(error, RedisCacheError::LiveClient(_)));
    assert!(harness.hooks.for_call("async_refresh_ttl").is_empty());
    let gate = harness.gate.entries();
    assert_eq!(gate[0], "enter:async_refresh_ttl");
    assert!(gate[1].starts_with("failure:"));
    assert_eq!(gate.len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_client_error_is_returned_and_recorded() {
    let harness = harness_at("127.0.0.1", 1, Some("ns"), 60).await;
    let error = harness
        .cache
        .async_refresh_ttl("alpha", Some(30))
        .await
        .expect_err("the client error");
    assert!(matches!(error, RedisCacheError::ClientReuse(_)));
    let gate = harness.gate.entries();
    assert_eq!(gate[0], "enter:async_refresh_ttl");
    assert!(gate[1].starts_with("failure:"));
    assert_eq!(gate.len(), 2);
}
