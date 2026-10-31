#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::SecretString;

use super::{PoolTarget, build_connection_pool};
use crate::redis_connection_settings::model::settings::{RedisConnectionSettings, Target};

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
    let pool = build_connection_pool(&RedisConnectionSettings::default(), Target::Direct, &env)
        .expect("a direct target builds a pool")
        .expect("a direct target gets a pool");
    assert_eq!(pool.timeout_seconds, 5);
}

#[test]
fn pool_timeout_reads_the_environment_override() {
    let mut env = BTreeMap::new();
    env.insert("REDIS_CONNECTION_POOL_TIMEOUT".to_owned(), "12".to_owned());
    let pool = build_connection_pool(&RedisConnectionSettings::default(), Target::Direct, &env)
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
    let error = build_connection_pool(&RedisConnectionSettings::default(), Target::Direct, &env)
        .expect_err("a non-numeric timeout cannot be parsed");
    assert!(error.to_string().contains("REDIS_CONNECTION_POOL_TIMEOUT"));
}

#[test]
fn a_direct_target_builds_a_pool() {
    let env = BTreeMap::<String, String>::new();
    let pool = build_connection_pool(&RedisConnectionSettings::default(), Target::Direct, &env)
        .expect("a direct target builds a pool")
        .expect("a direct target gets a pool");
    assert_eq!(pool.target, PoolTarget::Direct);
}

#[test]
fn a_url_target_builds_a_pool_carrying_the_url() {
    let settings = RedisConnectionSettings {
        url: Some(SecretString::from("redis://cache:6379".to_owned())),
        ..blank_settings()
    };
    let env = BTreeMap::<String, String>::new();
    let pool = build_connection_pool(&settings, Target::Url, &env)
        .expect("a url target builds a pool")
        .expect("a url target gets a pool");
    assert_eq!(pool.target, PoolTarget::Url);
    assert!(pool.url.is_some());
}

#[test]
fn a_cluster_target_never_gets_a_pool() {
    let env = BTreeMap::<String, String>::new();
    let pool = build_connection_pool(&RedisConnectionSettings::default(), Target::Cluster, &env)
        .expect("cluster pool construction does not error");
    assert!(pool.is_none());
}

#[test]
fn a_sentinel_target_never_gets_a_pool() {
    let env = BTreeMap::<String, String>::new();
    let pool = build_connection_pool(&RedisConnectionSettings::default(), Target::Sentinel, &env)
        .expect("sentinel pool construction does not error");
    assert!(pool.is_none());
}
