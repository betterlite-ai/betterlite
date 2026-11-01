#![cfg(test)]

use std::collections::BTreeMap;

use super::build_client;
use crate::redis_client_factories::RedisClient;
use crate::redis_connection_settings::model::nodes::ClusterNodeConfig;
use crate::redis_connection_settings::model::settings::{ClientKind, RedisConnectionSettings};

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
fn startup_nodes_dispatch_to_a_cluster_client() {
    let settings = RedisConnectionSettings {
        startup_nodes: Some(vec![ClusterNodeConfig {
            host: "node-a".to_owned(),
            port: 7000,
        }]),
        ..blank_settings()
    };
    assert!(matches!(
        build_client(&settings, ClientKind::Sync).expect("startup_nodes resolves"),
        RedisClient::Cluster(_)
    ));
    assert!(matches!(
        build_client(&settings, ClientKind::Async).expect("startup_nodes resolves"),
        RedisClient::Cluster(_)
    ));
}

#[test]
fn a_url_dispatches_to_a_url_client() {
    let settings = RedisConnectionSettings {
        url: Some(secrecy::SecretString::from("redis://cache:6379".to_owned())),
        ..blank_settings()
    };
    assert!(matches!(
        build_client(&settings, ClientKind::Sync).expect("url resolves"),
        RedisClient::Url(_)
    ));
}

#[test]
fn sentinel_nodes_and_service_name_dispatch_to_a_sentinel_client() {
    let settings = RedisConnectionSettings {
        sentinel_nodes: Some(vec![("sentinel-1".to_owned(), 26379)]),
        service_name: Some("mymaster".to_owned()),
        ..blank_settings()
    };
    assert!(matches!(
        build_client(&settings, ClientKind::Sync).expect("sentinel_nodes and service_name resolve"),
        RedisClient::Sentinel(_)
    ));
}

#[test]
fn a_bare_host_dispatches_to_a_direct_client() {
    let settings = RedisConnectionSettings {
        host: Some("cache".to_owned()),
        ..blank_settings()
    };
    assert!(matches!(
        build_client(&settings, ClientKind::Sync).expect("host resolves"),
        RedisClient::Direct(_)
    ));
}

#[test]
fn startup_nodes_take_priority_over_every_other_target() {
    let settings = RedisConnectionSettings {
        startup_nodes: Some(vec![ClusterNodeConfig {
            host: "node-a".to_owned(),
            port: 7000,
        }]),
        url: Some(secrecy::SecretString::from("redis://cache:6379".to_owned())),
        sentinel_nodes: Some(vec![("sentinel-1".to_owned(), 26379)]),
        service_name: Some("mymaster".to_owned()),
        host: Some("cache".to_owned()),
        ..blank_settings()
    };
    assert!(matches!(
        build_client(&settings, ClientKind::Sync).expect("startup_nodes resolves"),
        RedisClient::Cluster(_)
    ));
}

#[test]
fn an_async_cluster_dispatch_carries_isolation_but_a_sync_one_does_not() {
    let settings = RedisConnectionSettings {
        startup_nodes: Some(vec![ClusterNodeConfig {
            host: "node-a".to_owned(),
            port: 7000,
        }]),
        ..blank_settings()
    };
    let RedisClient::Cluster(sync_client) =
        build_client(&settings, ClientKind::Sync).expect("resolves")
    else {
        panic!("expected a cluster client");
    };
    let RedisClient::Cluster(async_client) =
        build_client(&settings, ClientKind::Async).expect("resolves")
    else {
        panic!("expected a cluster client");
    };
    assert!(sync_client.isolation.is_none());
    assert!(async_client.isolation.is_some());
}

#[test]
fn no_target_fields_set_is_an_error() {
    build_client(&blank_settings(), ClientKind::Sync).expect_err("no target field is set");
}
