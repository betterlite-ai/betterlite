use rstest::rstest;

use super::support::{DEFAULT_MAJOR_VERSION, TestGate, connect_cache, harness_at, overrides};
use crate::redis_live_client::tests::fake_server::{FakeServer, Reply};

#[rstest]
#[case::three_part_text("redis_version:7.2.4", 7)]
#[case::integer("redis_version:6", 6)]
#[case::float("redis_version:7.2", 7)]
#[case::unreadable_text("redis_version:latest", 7)]
#[case::unsupported_kind("redis_version:1,2", 7)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_info_reply_sets_the_major_version(#[case] version_line: &str, #[case] expected: u32) {
    let server = FakeServer::start().await;
    server.script(
        "INFO",
        Reply::Bulk(format!("# Server\r\n{version_line}\r\n")),
    );
    let harness = connect_cache(
        overrides(&server.host(), server.port()),
        None,
        60,
        DEFAULT_MAJOR_VERSION,
        TestGate::never(),
    )
    .await;
    assert_eq!(harness.cache.redis_major_version, expected);
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
