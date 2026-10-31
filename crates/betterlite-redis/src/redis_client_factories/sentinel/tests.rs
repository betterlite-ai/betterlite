#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::{ExposeSecret, SecretString};

use super::build;
use crate::redis_connection_settings::model::settings::{
    CredentialProviderSettings, RedisConnectionSettings,
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

fn sentinel_settings() -> RedisConnectionSettings {
    RedisConnectionSettings {
        sentinel_nodes: Some(vec![("sentinel-1".to_owned(), 26379)]),
        service_name: Some("mymaster".to_owned()),
        ..blank_settings()
    }
}

#[test]
fn sentinel_monitor_auth_never_carries_the_credential_provider_or_data_password() {
    let settings = RedisConnectionSettings {
        password: Some(SecretString::from("data-node-password".to_owned())),
        credential_provider: true,
        credential_provider_settings: Some(CredentialProviderSettings::Explicit),
        sentinel_password: Some(SecretString::from("sentinel-only-password".to_owned())),
        ..sentinel_settings()
    };
    let client = build(&settings).expect("sentinel_nodes and service_name are both set");

    assert_eq!(
        client
            .monitor_password
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("sentinel-only-password")
    );
    assert!(client.connection_kwargs.credential_provider);
    assert_eq!(
        client
            .connection_kwargs
            .password
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("data-node-password")
    );
}

#[test]
fn sentinel_monitor_password_is_absent_when_no_sentinel_password_was_configured() {
    let settings = RedisConnectionSettings {
        password: Some(SecretString::from("data-node-password".to_owned())),
        ..sentinel_settings()
    };
    let client = build(&settings).expect("sentinel_nodes and service_name are both set");
    assert!(client.monitor_password.is_none());
}

#[test]
fn sentinel_socket_timeout_defaults_to_0_1_seconds_when_unset() {
    let settings = sentinel_settings();
    let client = build(&settings).expect("sentinel_nodes and service_name are both set");
    assert_eq!(client.connection_kwargs.socket_timeout, Some(0.1_f64));
}

#[test]
fn sentinel_socket_timeout_keeps_an_explicit_value() {
    let settings = RedisConnectionSettings {
        socket_timeout: Some(2.5_f64),
        ..sentinel_settings()
    };
    let client = build(&settings).expect("sentinel_nodes and service_name are both set");
    assert_eq!(client.connection_kwargs.socket_timeout, Some(2.5_f64));
}

#[test]
fn sentinel_requires_both_nodes_and_service_name() {
    let missing_service_name = RedisConnectionSettings {
        sentinel_nodes: Some(vec![("sentinel-1".to_owned(), 26379)]),
        ..blank_settings()
    };
    build(&missing_service_name).expect_err("service_name is missing");

    let missing_nodes = RedisConnectionSettings {
        service_name: Some("mymaster".to_owned()),
        ..blank_settings()
    };
    build(&missing_nodes).expect_err("sentinel_nodes is missing");
}
