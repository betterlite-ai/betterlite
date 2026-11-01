#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::{ExposeSecret as _, SecretString};

use super::build;
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
fn builds_a_url_client_from_the_url_field() {
    let settings = RedisConnectionSettings {
        url: Some(SecretString::from("redis://cache:6379".to_owned())),
        max_connections: Some(10),
        ..blank_settings()
    };
    let client = build(&settings).expect("a url target builds");
    assert_eq!(client.url.expose_secret(), "redis://cache:6379");
    assert_eq!(client.max_connections, Some(10));
}

#[test]
fn a_missing_url_is_rejected() {
    let settings = blank_settings();
    let error = build(&settings).expect_err("no url cannot build a url client");
    assert_eq!(
        error.to_string(),
        "a url-targeted redis client requires 'url' to be set"
    );
}
