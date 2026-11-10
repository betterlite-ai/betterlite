use rstest::rstest;

use super::support::{DEFAULT_MAJOR_VERSION, TestGate, connect_cache, harness_at, overrides};
use crate::list_operations::results::LpopResult;
use crate::redis_live_client::tests::fake_server::{FakeServer, Reply};

#[rstest]
#[case::three_part_text("redis_version:7.2.4", 7, 5)]
#[case::integer("redis_version:6", 6, DEFAULT_MAJOR_VERSION)]
#[case::float("redis_version:6.5", 6, DEFAULT_MAJOR_VERSION)]
#[case::unreadable_text("redis_version:latest", DEFAULT_MAJOR_VERSION, DEFAULT_MAJOR_VERSION)]
#[case::unsupported_kind("redis_version:1,2", DEFAULT_MAJOR_VERSION, DEFAULT_MAJOR_VERSION)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_info_reply_sets_the_major_version(
    #[case] version_line: &str,
    #[case] expected_major: u32,
    #[case] default_major: u32,
) {
    let server = FakeServer::start().await;
    server.script(
        "INFO",
        Reply::Bulk(format!("# Server\r\n{version_line}\r\n")),
    );
    let harness = connect_cache(
        overrides(&server.host(), server.port()),
        None,
        60,
        default_major,
        TestGate::never(),
    )
    .await;
    assert_eq!(harness.cache.redis_major_version, expected_major);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_info_keeps_the_default_major_version() {
    let server = FakeServer::start().await;
    server.script("INFO", Reply::Error("ERR info is unavailable".to_owned()));
    let harness = connect_cache(
        overrides(&server.host(), server.port()),
        None,
        60,
        6,
        TestGate::never(),
    )
    .await;
    assert_eq!(harness.cache.redis_major_version, 6);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_info_reply_without_a_version_keeps_the_default_major_version() {
    let server = FakeServer::start().await;
    server.script(
        "INFO",
        Reply::Bulk("# Server\r\nuptime_in_seconds:12\r\n".to_owned()),
    );
    let harness = connect_cache(
        overrides(&server.host(), server.port()),
        None,
        60,
        6,
        TestGate::never(),
    )
    .await;
    assert_eq!(harness.cache.redis_major_version, 6);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unreachable_redis_still_yields_a_cache_with_the_default_major_version() {
    let harness = harness_at("127.0.0.1", 1, None, 60).await;
    assert_eq!(harness.cache.redis_major_version, DEFAULT_MAJOR_VERSION);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_older_reported_version_reaches_the_list_slice_and_pops_one_by_one() {
    let server = FakeServer::start().await;
    server.script(
        "INFO",
        Reply::Bulk("# Server\r\nredis_version:6.0.9\r\n".to_owned()),
    );
    server.script("LPOP", Reply::Bulk("one".to_owned()));
    server.script("LPOP", Reply::Bulk("two".to_owned()));
    let harness = connect_cache(
        overrides(&server.host(), server.port()),
        None,
        60,
        7,
        TestGate::never(),
    )
    .await;
    let popped = harness
        .cache
        .lists
        .async_lpop("alpha", Some(2))
        .await
        .expect("the pop");
    assert_eq!(
        popped,
        LpopResult::Items(vec!["one".to_owned(), "two".to_owned()])
    );
    let commands = server.commands_named("LPOP");
    assert_eq!(commands.len(), 2);
    assert!(
        commands
            .iter()
            .all(|command| command.args == ["LPOP", "alpha"])
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_newer_reported_version_reaches_the_list_slice_and_pops_with_a_count() {
    let server = FakeServer::start().await;
    server.script(
        "INFO",
        Reply::Bulk("# Server\r\nredis_version:7.2.4\r\n".to_owned()),
    );
    server.script(
        "LPOP",
        Reply::Array(vec![
            Reply::Bulk("one".to_owned()),
            Reply::Bulk("two".to_owned()),
        ]),
    );
    let harness = connect_cache(
        overrides(&server.host(), server.port()),
        None,
        60,
        6,
        TestGate::never(),
    )
    .await;
    let popped = harness
        .cache
        .lists
        .async_lpop("alpha", Some(2))
        .await
        .expect("the pop");
    assert_eq!(
        popped,
        LpopResult::Items(vec!["one".to_owned(), "two".to_owned()])
    );
    let commands = server.commands_named("LPOP");
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].args, ["LPOP", "alpha", "2"]);
}
