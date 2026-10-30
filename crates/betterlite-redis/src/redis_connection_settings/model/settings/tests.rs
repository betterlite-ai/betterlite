#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::{ExposeSecret, SecretString};

use super::{RedisConnectionSettings, RedisOverrides};

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
        startup_nodes: None,
        sentinel_nodes: None,
        sentinel_password: None,
        service_name: None,
        passthrough: BTreeMap::new(),
    }
}

#[test]
fn overrides_default_to_every_field_absent() {
    let overrides = RedisOverrides::default();
    assert!(overrides.host.is_none());
    assert!(overrides.url.is_none());
    assert!(!overrides.credential_provider);
    assert!(overrides.passthrough.is_empty());
}

#[test]
fn overrides_deserialize_from_a_sparse_object() {
    let overrides: RedisOverrides = serde_json::from_str(r#"{"host": "cache.internal"}"#)
        .expect("a sparse object deserializes using field defaults");
    assert_eq!(overrides.host, Some("cache.internal".to_owned()));
    assert!(overrides.port.is_none());
}

#[test]
fn password_round_trips_through_expose_secret() {
    let secret = SecretString::from("s3cret".to_owned());
    let settings = RedisConnectionSettings {
        password: Some(secret),
        ..blank_settings()
    };
    assert_eq!(
        settings.password.as_ref().map(ExposeSecret::expose_secret),
        Some("s3cret")
    );
}
