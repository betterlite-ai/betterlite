#![cfg(test)]

use std::collections::BTreeMap;

use super::build;
use crate::redis_cluster_node_isolation::IsolationBranch;
use crate::redis_connection_settings::model::nodes::ClusterNodeConfig;
use crate::redis_connection_settings::model::scalars::PassthroughValue;
use crate::redis_connection_settings::model::settings::{ClientKind, RedisConnectionSettings};

fn cluster_settings() -> RedisConnectionSettings {
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
        startup_nodes: Some(vec![
            ClusterNodeConfig {
                host: "node-a".to_owned(),
                port: 7000,
            },
            ClusterNodeConfig {
                host: "node-b".to_owned(),
                port: 7001,
            },
        ]),
        sentinel_nodes: None,
        sentinel_password: None,
        service_name: None,
        ssl_password: None,
        passthrough: BTreeMap::new(),
    }
}

#[test]
fn sync_cluster_clients_carry_no_isolation_transport() {
    let client = build(&cluster_settings(), ClientKind::Sync);
    assert!(client.isolation.is_none());
}

#[test]
fn async_cluster_applies_the_per_node_recovery_isolation_branch() {
    let client = build(&cluster_settings(), ClientKind::Async);
    let isolation = client.isolation.expect("async cluster carries isolation");
    assert_eq!(isolation.branch, IsolationBranch::PerNodeRecovery);
}

#[test]
fn async_cluster_isolation_reads_reinitialize_steps_from_passthrough() {
    let mut settings = cluster_settings();
    settings.passthrough.insert(
        "reinitialize_steps".to_owned(),
        PassthroughValue::Number(serde_json::Number::from(3_u32)),
    );
    let client = build(&settings, ClientKind::Async);
    let isolation = client.isolation.expect("async cluster carries isolation");
    assert_eq!(isolation.reinitialize_steps, 3);
}

#[test]
fn async_cluster_health_interval_defaults_to_25_seconds_and_keepalive_defaults_to_true() {
    let client = build(&cluster_settings(), ClientKind::Async);
    assert_eq!(client.health_check_interval, Some(25));
    assert_eq!(client.socket_keepalive, Some(true));
}

#[test]
fn async_cluster_health_interval_and_keepalive_keep_an_explicit_override() {
    let settings = RedisConnectionSettings {
        health_check_interval: Some(60),
        socket_keepalive: Some(false),
        ..cluster_settings()
    };
    let client = build(&settings, ClientKind::Async);
    assert_eq!(client.health_check_interval, Some(60));
    assert_eq!(client.socket_keepalive, Some(false));
}

#[test]
fn sync_cluster_health_interval_has_no_async_default() {
    let client = build(&cluster_settings(), ClientKind::Sync);
    assert!(client.health_check_interval.is_none());
}

#[test]
fn async_cluster_drops_redis_connect_func_even_without_a_credential_provider() {
    let mut settings = cluster_settings();
    settings.passthrough.insert(
        "redis_connect_func".to_owned(),
        PassthroughValue::Text("iam-callback".to_owned()),
    );
    let client = build(&settings, ClientKind::Async);
    assert!(!client.passthrough.contains_key("redis_connect_func"));
}

#[test]
fn sync_cluster_keeps_redis_connect_func() {
    let mut settings = cluster_settings();
    settings.passthrough.insert(
        "redis_connect_func".to_owned(),
        PassthroughValue::Text("iam-callback".to_owned()),
    );
    let client = build(&settings, ClientKind::Sync);
    assert!(client.passthrough.contains_key("redis_connect_func"));
}
