use std::io;
use std::sync::Arc;

use rstest::rstest;

use super::support::cluster::{cluster_pair, cluster_settings};
use super::support::{Gate, Hooks, Logs, Probe, direct_settings, empty_settings, probe};
use crate::redis_cache_admin::connection_test::{ConnectionStatus, connection_test_result};
use crate::redis_cache_admin::{RedisAdminError, RedisCacheAdmin};
use crate::redis_client_reuse::store::RedisClientStore;
use crate::redis_client_reuse::transport::{RedisCacheFamily, RedisTransport};
use crate::redis_live_client::LiveClientError;
use crate::redis_live_client::LiveClientFactory;
use crate::redis_live_client::tests::fake_server::{FakeServer, Reply};

fn standalone(server: &FakeServer) -> Probe {
    probe(
        &direct_settings(&server.host(), server.port()),
        RedisCacheFamily::Standalone,
        None,
    )
}

fn admin_error(message: &str) -> RedisAdminError {
    RedisAdminError::from(LiveClientError::Redis(redis::RedisError::from(
        io::Error::other(message.to_owned()),
    )))
}

#[rstest]
#[case::standalone_success(
    RedisCacheFamily::Standalone,
    Ok(true),
    ConnectionStatus::Success,
    "Redis connection test successful",
    None
)]
#[case::standalone_false(
    RedisCacheFamily::Standalone,
    Ok(false),
    ConnectionStatus::Failed,
    "Redis ping returned False",
    None
)]
#[case::standalone_error(
    RedisCacheFamily::Standalone,
    Err(admin_error("down")),
    ConnectionStatus::Failed,
    "Redis connection failed: down",
    Some("down")
)]
#[case::cluster_success(
    RedisCacheFamily::Cluster,
    Ok(true),
    ConnectionStatus::Success,
    "Redis Cluster connection test successful",
    None
)]
#[case::cluster_false(
    RedisCacheFamily::Cluster,
    Ok(false),
    ConnectionStatus::Failed,
    "Redis Cluster ping returned False",
    None
)]
#[case::cluster_error(
    RedisCacheFamily::Cluster,
    Err(admin_error("down")),
    ConnectionStatus::Failed,
    "Redis Cluster connection failed: down",
    Some("down")
)]
fn connection_test_result_texts(
    #[case] family: RedisCacheFamily,
    #[case] ping: Result<bool, RedisAdminError>,
    #[case] status: ConnectionStatus,
    #[case] message: &str,
    #[case] error: Option<&str>,
) {
    let result = connection_test_result(family, ping);
    assert_eq!(result.status, status);
    assert_eq!(result.message, message);
    assert_eq!(result.error.as_deref(), error);
}

#[test]
fn the_result_serializes_the_status_message_and_optional_error() {
    let success = serde_json::to_string(&connection_test_result(
        RedisCacheFamily::Standalone,
        Ok(true),
    ))
    .expect("a serialized success");
    assert!(success.contains(r#""status":"success""#));
    assert!(success.contains(r#""message":"Redis connection test successful""#));
    assert!(!success.contains(r#""error":"#));
    let failure = serde_json::to_string(&connection_test_result(
        RedisCacheFamily::Cluster,
        Err(admin_error("down")),
    ))
    .expect("a serialized failure");
    assert!(failure.contains(r#""status":"failed""#));
    assert!(failure.contains(r#""message":"Redis Cluster connection failed: down""#));
    assert!(failure.contains(r#""error":"down""#));
}

#[tokio::test]
async fn opens_a_fresh_client_for_each_connection_test() {
    let server = FakeServer::start().await;
    let fixture = standalone(&server);
    let first = fixture.admin.test_connection().await;
    let second = fixture.admin.test_connection().await;
    assert_eq!(first.status, ConnectionStatus::Success);
    assert_eq!(second.status, ConnectionStatus::Success);
    assert_eq!(server.commands_named("PING").len(), 2);
    assert_eq!(server.connections(), 2);
    assert!(fixture.hooks.successes().is_empty());
    assert!(fixture.hooks.failures().is_empty());
}

#[tokio::test]
async fn reports_a_false_ping_without_an_error() {
    let server = FakeServer::start().await;
    server.script("PING", Reply::Simple("NOPE".to_owned()));
    let fixture = standalone(&server);
    let result = fixture.admin.test_connection().await;
    assert_eq!(result.status, ConnectionStatus::Failed);
    assert_eq!(result.message, "Redis ping returned False");
    assert_eq!(result.error, None);
}

#[tokio::test]
async fn reports_a_ping_error_and_logs_it() {
    let server = FakeServer::start().await;
    server.script("PING", Reply::Error("ERR tls handshake failed".to_owned()));
    let (logs, _guard) = Logs::capture();
    let fixture = standalone(&server);
    let result = fixture.admin.test_connection().await;
    assert_eq!(result.status, ConnectionStatus::Failed);
    assert!(result.message.contains("Redis connection failed: "));
    assert!(
        result
            .error
            .is_some_and(|error| error.contains("tls handshake failed"))
    );
    let lines = logs.lines();
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Redis connection test failed: "))
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("tls handshake failed"))
    );
}

#[tokio::test]
async fn reports_a_failed_client_build_without_failing() {
    let store = Arc::new(RedisClientStore::new());
    let factory = Arc::new(LiveClientFactory::new(None));
    let mut transport = RedisTransport::new(
        direct_settings("127.0.0.1", 6379),
        RedisCacheFamily::Standalone,
        Arc::clone(&factory),
        Arc::clone(&store),
        None,
    )
    .expect("a transport");
    // the transport carries the settings test_connection builds its fresh client from
    transport.settings = empty_settings();
    let admin = RedisCacheAdmin::new(
        Arc::new(transport),
        None,
        Arc::new(Hooks::default()),
        Arc::new(Gate::default()),
    );
    let result = admin.test_connection().await;
    assert_eq!(result.status, ConnectionStatus::Failed);
    assert!(result.message.starts_with("Redis connection failed: "));
    let error = result.error.expect("the build error text");
    assert!(error.contains("Either 'host' or 'url' must be specified for redis."));
}

#[tokio::test]
async fn uses_the_cluster_texts() {
    let (first, second) = cluster_pair().await;
    let settings =
        cluster_settings(&[(first.host(), first.port()), (second.host(), second.port())]);
    let fixture = probe(&settings, RedisCacheFamily::Cluster, None);
    let result = fixture.admin.test_connection().await;
    assert_eq!(result.status, ConnectionStatus::Success);
    assert_eq!(result.message, "Redis Cluster connection test successful");
}
