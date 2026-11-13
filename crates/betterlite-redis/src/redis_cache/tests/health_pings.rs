use super::support::{DEFAULT_MAJOR_VERSION, harness};
use crate::redis_live_client::tests::fake_server::{FakeServer, Reply};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connect_starts_the_health_pings() {
    let server = FakeServer::start().await;
    let _harness = harness(&server, None, 60).await;
    assert!(!server.commands_named("PING").is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failing_ping_still_returns_a_cache() {
    let server = FakeServer::start().await;
    server.script("PING", Reply::Error("ERR no ping".to_owned()));
    server.script("PING", Reply::Error("ERR no ping".to_owned()));
    let harness = harness(&server, None, 60).await;
    assert_eq!(harness.cache.redis_major_version, DEFAULT_MAJOR_VERSION);
}
