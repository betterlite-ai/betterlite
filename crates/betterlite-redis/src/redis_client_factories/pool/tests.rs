#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};

use super::{PoolConnection, build_connection_pool};
use crate::redis_connection_settings::model::scalars::PassthroughValue;
use crate::redis_connection_settings::model::settings::{
    CredentialProviderSettings, RedisConnectionSettings, Target,
};

fn blank_settings() -> RedisConnectionSettings {
    RedisConnectionSettings {
        host: None,
        port: None,
        db: None,
        username: None,
        password: None,
        url: None,
        ssl: None,
        socket_timeout: None,
        socket_connect_timeout: None,
        socket_keepalive: None,
        max_connections: None,
        health_check_interval: None,
        credential_provider: false,
        credential_provider_settings: None,
        startup_nodes: None,
        sentinel_nodes: None,
        sentinel_password: None,
        service_name: None,
        ssl_password: None,
        passthrough: BTreeMap::new(),
    }
}

fn build_test_pool(target: Target, max_connections: Option<i64>) -> super::PoolConfig {
    let settings = RedisConnectionSettings {
        url: (target == Target::Url).then(|| SecretString::from("redis://cache:6379".to_owned())),
        max_connections,
        ..blank_settings()
    };
    build_connection_pool(&settings, target, &BTreeMap::<String, String>::new())
        .expect("pool builds")
        .expect("pool exists")
}

#[test]
fn pool_timeout_defaults_to_five_seconds() {
    let env = BTreeMap::<String, String>::new();
    let pool = build_connection_pool(&blank_settings(), Target::Direct, &env)
        .expect("a direct target builds a pool")
        .expect("a direct target gets a pool");
    assert_eq!(pool.timeout_seconds, 5);
}

#[test]
fn pool_timeout_reads_the_environment_override() {
    let mut env = BTreeMap::new();
    env.insert("REDIS_CONNECTION_POOL_TIMEOUT".to_owned(), "12".to_owned());
    let pool = build_connection_pool(&blank_settings(), Target::Direct, &env)
        .expect("a direct target builds a pool")
        .expect("a direct target gets a pool");
    assert_eq!(pool.timeout_seconds, 12);
}

#[test]
fn an_invalid_pool_timeout_is_an_error() {
    let mut env = BTreeMap::new();
    env.insert(
        "REDIS_CONNECTION_POOL_TIMEOUT".to_owned(),
        "soon".to_owned(),
    );
    let error = build_connection_pool(&blank_settings(), Target::Direct, &env)
        .expect_err("a non-numeric timeout cannot be parsed");
    assert!(error.to_string().contains("REDIS_CONNECTION_POOL_TIMEOUT"));
}

#[test]
fn a_direct_target_builds_a_pool_carrying_host_port_password_and_socket_timeout() {
    let settings = RedisConnectionSettings {
        host: Some("cache.internal".to_owned()),
        port: Some(6380),
        password: Some(SecretString::from("hunter2".to_owned())),
        socket_timeout: Some(3.5_f64),
        ..blank_settings()
    };
    let env = BTreeMap::<String, String>::new();
    let pool = build_connection_pool(&settings, Target::Direct, &env)
        .expect("a direct target builds a pool")
        .expect("a direct target gets a pool");
    let PoolConnection::Direct(fields) = pool.connection else {
        panic!("expected a direct pool connection");
    };
    assert_eq!(fields.host.as_deref(), Some("cache.internal"));
    assert_eq!(fields.port, Some(6380));
    assert_eq!(
        fields.password.as_ref().map(ExposeSecret::expose_secret),
        Some("hunter2")
    );
    assert_eq!(fields.socket_timeout, Some(3.5_f64));
}

#[test]
fn a_url_target_builds_a_pool_carrying_the_url_socket_timeout_and_credential_provider() {
    let settings = RedisConnectionSettings {
        url: Some(SecretString::from("redis://cache:6379".to_owned())),
        socket_timeout: Some(2.0_f64),
        credential_provider: true,
        credential_provider_settings: Some(CredentialProviderSettings::Explicit),
        ..blank_settings()
    };
    let env = BTreeMap::<String, String>::new();
    let pool = build_connection_pool(&settings, Target::Url, &env)
        .expect("a url target builds a pool")
        .expect("a url target gets a pool");
    let PoolConnection::Url(client) = pool.connection else {
        panic!("expected a url pool connection");
    };
    assert_eq!(client.url.expose_secret(), "redis://cache:6379");
    assert_eq!(client.socket_timeout, Some(2.0_f64));
    assert!(client.credential_provider);
}

#[test]
fn a_cluster_target_never_gets_a_pool() {
    let env = BTreeMap::<String, String>::new();
    let pool = build_connection_pool(&blank_settings(), Target::Cluster, &env)
        .expect("cluster pool construction does not error");
    assert!(pool.is_none());
}

#[test]
fn a_sentinel_target_never_gets_a_pool() {
    let env = BTreeMap::<String, String>::new();
    let pool = build_connection_pool(&blank_settings(), Target::Sentinel, &env)
        .expect("sentinel pool construction does not error");
    assert!(pool.is_none());
}

#[rstest]
#[case::direct(Target::Direct)]
#[case::url(Target::Url)]
fn pool_size_defaults_to_50_connections(#[case] target: Target) {
    let pool = build_test_pool(target, None);
    assert_eq!(pool.max_connections, 50);
}

#[rstest]
#[case::direct(Target::Direct)]
#[case::url(Target::Url)]
fn pool_size_follows_the_configured_max_connections(#[case] target: Target) {
    let pool = build_test_pool(target, Some(12));
    assert_eq!(pool.max_connections, 12);
    match pool.connection {
        PoolConnection::Direct(fields) => assert_eq!(fields.max_connections, Some(12)),
        PoolConnection::Url(client) => assert_eq!(client.max_connections, Some(12)),
    }
}

#[rstest]
#[case::zero(0)]
#[case::negative(-3)]
fn pool_size_rejects_max_connections_below_one(#[case] value: i64) {
    let settings = RedisConnectionSettings {
        max_connections: Some(value),
        ..blank_settings()
    };
    let error = build_connection_pool(
        &settings,
        Target::Direct,
        &BTreeMap::<String, String>::new(),
    )
    .expect_err("invalid size rejected");
    assert!(error.to_string().contains(&value.to_string()));
}

#[test]
fn url_pool_takes_tls_from_the_scheme_alone() {
    let settings = RedisConnectionSettings {
        url: Some(SecretString::from("redis://cache:6379".to_owned())),
        ssl: Some(true),
        ssl_password: Some(SecretString::from("secret".to_owned())),
        ..blank_settings()
    };
    let pool = build_connection_pool(&settings, Target::Url, &BTreeMap::<String, String>::new())
        .expect("pool builds")
        .expect("pool exists");
    let PoolConnection::Url(client) = pool.connection else {
        panic!("expected url pool")
    };
    assert_eq!(client.url.expose_secret(), "redis://cache:6379");
    assert!(!format!("{client:?}").contains("ssl"));
}

#[rstest]
#[case::direct_gcp(Target::Direct, CredentialProviderSettings::Gcp { service_account: "svc".to_owned(), ssl_ca_certs: None })]
#[case::url_azure(Target::Url, CredentialProviderSettings::Azure { client_id: None, tenant_id: None, client_secret: None })]
fn iam_provider_pool_replaces_static_credentials(
    #[case] target: Target,
    #[case] provider: CredentialProviderSettings,
) {
    let passthrough = BTreeMap::from([(
        "redis_connect_func".to_owned(),
        PassthroughValue::Text("callback".to_owned()),
    )]);
    let settings = RedisConnectionSettings {
        url: (target == Target::Url)
            .then(|| SecretString::from("redis://user:pass@cache:6379".to_owned())),
        username: Some("user".to_owned()),
        password: Some(SecretString::from("pass".to_owned())),
        credential_provider_settings: Some(provider),
        passthrough,
        ..blank_settings()
    };
    let pool = build_connection_pool(&settings, target, &BTreeMap::<String, String>::new())
        .expect("pool builds")
        .expect("pool exists");
    match pool.connection {
        PoolConnection::Direct(fields) => {
            assert!(fields.credential_provider);
            assert!(fields.username.is_none());
            assert!(fields.password.is_none());
            assert!(!fields.passthrough.contains_key("redis_connect_func"));
        }
        PoolConnection::Url(client) => {
            assert!(client.credential_provider);
            assert!(client.username.is_none());
            assert!(!client.passthrough.contains_key("redis_connect_func"));
            assert!(client.url.expose_secret().contains("user:pass@"));
        }
    }
}
