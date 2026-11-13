use std::collections::BTreeMap;

use betterlite_config::sections::routing_state_store::connection::Connection;

use super::super::error::{StateStoreError, StoreWarning};
use super::super::redis_state::connection_settings;
use super::super::{RoutingStateStore, Settings};
use super::support::{TestClock, breaker, route_key, same_f64, secret, settings};
use crate::routing_state_store::test_support::FakeRedis;
use crate::routing_value_store::StoreRead;

pub(crate) async fn run(name: &str) {
    match name {
        "connection_options_reach_the_handshake" => connection_options_reach_the_handshake().await,
        "connection_options_outrank_the_environment" => {
            connection_options_outrank_the_environment().await;
        }
        "ssl_option_switches_to_tls" => ssl_option_switches_to_tls().await,
        "connection_options_reach_the_client_settings" => {
            connection_options_reach_the_client_settings().await;
        }
        "iam_environment_needs_a_credential_source" => {
            iam_environment_needs_a_credential_source().await;
        }
        other => panic!("unknown case {other}"),
    }
}

fn handshake(arguments: &[&str]) -> Vec<String> {
    arguments.iter().map(|text| (*text).to_owned()).collect()
}

fn connection(db: Option<i64>, username: Option<&str>, ssl: Option<bool>) -> Connection {
    Connection {
        db,
        username: username.map(str::to_owned),
        ssl,
        socket_connect_timeout: None,
        socket_keepalive: None,
        max_connections: None,
        health_check_interval: None,
    }
}

async fn read_once(settings: &Settings, clock: &TestClock) -> Option<StoreWarning> {
    let store = RoutingStateStore::new(settings, clock.breaker_clock()).unwrap();
    let observed = store.get(&route_key("counter")).await.unwrap();
    assert_eq!(observed.value, StoreRead::Missing);
    observed.warning
}

async fn connection_options_reach_the_handshake() {
    let server = FakeRedis::start().await;
    let mut settings = settings(&server, None, breaker(5, 60_000));
    settings.redis.password = Some(secret("cache-password"));
    settings.redis.connection = connection(Some(2), Some("cache-user"), None);
    assert!(read_once(&settings, &TestClock::new()).await.is_none());
    let handshakes = server.handshakes();
    assert!(
        handshakes.contains(&handshake(&["AUTH", "cache-user", "cache-password"])),
        "{handshakes:?}"
    );
    assert!(
        handshakes.contains(&handshake(&["SELECT", "2"])),
        "{handshakes:?}"
    );
}

async fn connection_options_outrank_the_environment() {
    let server = FakeRedis::start().await;
    let mut settings = settings(&server, None, breaker(5, 60_000));
    settings.redis.connection = connection(Some(2), None, None);
    settings.redis.environment = BTreeMap::from([("REDIS_DB".to_owned(), secret("3"))]);
    assert!(read_once(&settings, &TestClock::new()).await.is_none());
    let handshakes = server.handshakes();
    assert!(
        handshakes.contains(&handshake(&["SELECT", "2"])),
        "{handshakes:?}"
    );
    assert!(
        !handshakes.contains(&handshake(&["SELECT", "3"])),
        "{handshakes:?}"
    );
}

async fn ssl_option_switches_to_tls() {
    let plain = FakeRedis::start().await;
    let plain_settings = settings(&plain, None, breaker(5, 60_000));
    assert!(
        read_once(&plain_settings, &TestClock::new())
            .await
            .is_none()
    );
    let secure = FakeRedis::start().await;
    let mut secure_settings = settings(&secure, None, breaker(5, 60_000));
    secure_settings.redis.connection = connection(None, None, Some(true));
    let warning = read_once(&secure_settings, &TestClock::new()).await;
    assert!(
        matches!(warning, Some(StoreWarning::Unavailable { .. })),
        "{warning:?}"
    );
}

async fn connection_options_reach_the_client_settings() {
    let server = FakeRedis::start().await;
    let mut settings = settings(&server, None, breaker(5, 60_000));
    settings.redis.connection = Connection {
        db: Some(4),
        username: Some("cache-user".to_owned()),
        ssl: Some(true),
        socket_connect_timeout: Some(2.5_f64),
        socket_keepalive: Some(true),
        max_connections: Some(10),
        health_check_interval: Some(30),
    };
    let resolved = connection_settings(&settings.redis).unwrap();
    assert_eq!(resolved.db, Some(4));
    assert_eq!(resolved.username.as_deref(), Some("cache-user"));
    assert_eq!(resolved.ssl, Some(true));
    assert!(
        resolved
            .socket_connect_timeout
            .is_some_and(|seconds| same_f64(seconds, 2.5_f64))
    );
    assert_eq!(resolved.socket_keepalive, Some(true));
    assert_eq!(resolved.max_connections, Some(10));
    assert_eq!(resolved.health_check_interval, Some(30));
}

async fn iam_environment_needs_a_credential_source() {
    let server = FakeRedis::start().await;
    let mut settings = settings(&server, None, breaker(5, 60_000));
    settings.redis.environment = BTreeMap::from([(
        "REDIS_GCP_SERVICE_ACCOUNT".to_owned(),
        secret("router@project.iam.gserviceaccount.com"),
    )]);
    let Err(error) = RoutingStateStore::new(&settings, TestClock::new().breaker_clock()) else {
        panic!("a configured credential provider must not connect without a credential");
    };
    assert!(
        matches!(
            error,
            StateStoreError::Redis {
                operation: "open",
                ..
            }
        ),
        "{error:?}"
    );
    assert!(server.handshakes().is_empty());
}
