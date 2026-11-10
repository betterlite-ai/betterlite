use rstest::rstest;

use super::support::{harness, harness_at};
use crate::redis_live_client::tests::fake_server::{FakeServer, Reply};

#[rstest]
#[case::seconds_left(30, Some(30))]
#[case::zero_stays_zero(0, Some(0))]
#[case::minus_one_is_a_miss(-1, None)]
#[case::minus_two_is_a_miss(-2, None)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_reply_becomes_the_remaining_ttl(#[case] reply: i64, #[case] expected: Option<i64>) {
    let server = FakeServer::start().await;
    server.script("TTL", Reply::Int(reply));
    let harness = harness(&server, Some("ns"), 60).await;
    let remaining = harness
        .cache
        .async_get_ttl("alpha")
        .await
        .expect("the ttl read");
    assert_eq!(remaining, expected);
    assert_eq!(server.commands_named("TTL")[0].args, ["TTL", "ns:alpha"]);
    assert_eq!(harness.gate.entries(), ["enter:async_get_ttl", "exit:1"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_error_is_swallowed_into_none_without_a_hook() {
    let server = FakeServer::start().await;
    server.script("TTL", Reply::Error("ERR ttl failed".to_owned()));
    let harness = harness(&server, Some("ns"), 60).await;
    let remaining = harness
        .cache
        .async_get_ttl("alpha")
        .await
        .expect("the swallow");
    assert_eq!(remaining, None);
    assert!(harness.hooks.for_call("async_get_ttl").is_empty());
    let gate = harness.gate.entries();
    assert_eq!(gate[0], "enter:async_get_ttl");
    assert!(gate[1].starts_with("swallowed:1:"));
    assert_eq!(gate[2], "exit:1");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_client_error_is_swallowed_into_none_without_a_hook() {
    let harness = harness_at("127.0.0.1", 1, Some("ns"), 60).await;
    let remaining = harness
        .cache
        .async_get_ttl("alpha")
        .await
        .expect("the swallow");
    assert_eq!(remaining, None);
    assert!(harness.hooks.for_call("async_get_ttl").is_empty());
    let gate = harness.gate.entries();
    assert_eq!(gate[0], "enter:async_get_ttl");
    assert!(gate[1].starts_with("swallowed:1:"));
    assert_eq!(gate[2], "exit:1");
}
