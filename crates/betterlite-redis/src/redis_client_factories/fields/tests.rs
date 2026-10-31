#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::{ExposeSecret, SecretString};

use super::ConnectionFields;
use crate::redis_connection_settings::model::settings::RedisConnectionSettings;

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
fn carries_host_and_secret_fields_across() {
    let settings = RedisConnectionSettings {
        host: Some("cache.internal".to_owned()),
        port: Some(6379),
        password: Some(SecretString::from("s3cret".to_owned())),
        credential_provider: true,
        ..blank_settings()
    };
    let fields = ConnectionFields::from(&settings);
    assert_eq!(fields.host, Some("cache.internal".to_owned()));
    assert_eq!(fields.port, Some(6379));
    assert_eq!(
        fields.password.as_ref().map(ExposeSecret::expose_secret),
        Some("s3cret")
    );
    assert!(fields.credential_provider);
}
