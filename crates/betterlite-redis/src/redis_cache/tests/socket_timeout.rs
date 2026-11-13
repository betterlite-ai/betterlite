use super::support::{
    DEFAULT_MAJOR_VERSION, TestGate, connect_cache, harness, overrides_without_socket_timeout,
};
use crate::redis_live_client::tests::fake_server::FakeServer;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_missing_socket_timeout_becomes_five_seconds() {
    let server = FakeServer::start().await;
    let harness = connect_cache(
        overrides_without_socket_timeout(&server.host(), server.port()),
        None,
        60,
        DEFAULT_MAJOR_VERSION,
        TestGate::never(),
    )
    .await;
    assert_eq!(
        harness.cache.transport.settings.socket_timeout,
        Some(5.0_f64)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_given_socket_timeout_stays() {
    let server = FakeServer::start().await;
    let harness = harness(&server, None, 60).await;
    assert_eq!(
        harness.cache.transport.settings.socket_timeout,
        Some(1.0_f64)
    );
}
