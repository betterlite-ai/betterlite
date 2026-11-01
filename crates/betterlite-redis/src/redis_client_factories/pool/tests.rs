#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::{ExposeSecret, SecretString};

use super::{PoolConnection, build_connection_pool};
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
