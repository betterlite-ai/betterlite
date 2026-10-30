#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};

use super::{resolve_redis_kwargs, settings_uses_tls};
use crate::redis_connection_settings::model::nodes::{ClusterNodeConfig, NodesInput};
use crate::redis_connection_settings::model::scalars::PassthroughValue;
use crate::redis_connection_settings::model::settings::{
    ClientKind, RedisConnectionSettings, RedisOverrides,
};

fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn blank_overrides() -> RedisOverrides {
    RedisOverrides {
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
fn redis_coerce_turns_environment_strings_into_typed_fields() {
    let env = env(&[
        ("REDIS_HOST", "cache.internal"),
        ("REDIS_PORT", "6380"),
        ("REDIS_SOCKET_TIMEOUT", "2.5"),
        ("REDIS_SSL", "true"),
    ]);
    let settings = resolve_redis_kwargs(RedisOverrides::default(), ClientKind::Sync, &env)
        .expect("host names a target");
    assert_eq!(settings.port, Some(6380));
    assert_eq!(settings.socket_timeout, Some(2.5_f64));
    assert_eq!(settings.ssl, Some(true));
}

#[test]
fn redis_coerce_reads_signature_kwargs_from_environment() {
    let env = env(&[
        ("REDIS_HOST", "cache.internal"),
        ("REDIS_RETRY_ON_TIMEOUT", "false"),
        ("REDIS_SSL_CA_CERTS", "/etc/redis/ca.pem"),
    ]);
    let settings = resolve_redis_kwargs(RedisOverrides::default(), ClientKind::Sync, &env)
        .expect("host names a target");
    assert_eq!(
        settings.passthrough["retry_on_timeout"],
        PassthroughValue::Bool(false)
    );
    assert_eq!(
        settings.passthrough["ssl_ca_certs"],
        PassthroughValue::Text("/etc/redis/ca.pem".to_owned())
    );
}

#[test]
fn redis_precedence_keeps_explicit_connection_options() {
    let overrides: RedisOverrides = serde_json::from_str(
        r#"{"host":"cache.internal","client_name":"configured","retry_on_timeout":"false"}"#,
    )
    .unwrap();
    let environment = env(&[
        ("REDIS_CLIENT_NAME", "environment"),
        ("REDIS_RETRY_ON_TIMEOUT", "true"),
    ]);
    let settings = resolve_redis_kwargs(overrides, ClientKind::Sync, &environment).unwrap();
    assert_eq!(
        settings.passthrough["client_name"],
        PassthroughValue::Text("configured".to_owned())
    );
    assert_eq!(
        settings.passthrough["retry_on_timeout"],
        PassthroughValue::Bool(false)
    );
}

#[test]
fn redis_precedence_uses_environment_nodes_for_an_empty_override() {
    let overrides = RedisOverrides {
        host: Some("cache.internal".to_owned()),
        startup_nodes: Some(NodesInput::Text(String::new())),
        ..blank_overrides()
    };
    let env = env(&[("REDIS_CLUSTER_NODES", r#"[{"host":"a","port":7000}]"#)]);
    let settings = resolve_redis_kwargs(overrides, ClientKind::Sync, &env)
        .expect("environment startup_nodes names a target");
    assert_eq!(settings.startup_nodes.map(|nodes| nodes.len()), Some(1));
}

#[test]
fn redis_coerce_drops_a_value_that_does_not_parse() {
    let env = env(&[
        ("REDIS_HOST", "cache.internal"),
        ("REDIS_PORT", "not-a-port"),
    ]);
    let settings = resolve_redis_kwargs(RedisOverrides::default(), ClientKind::Sync, &env)
        .expect("host names a target");
    assert_eq!(settings.port, None);
}

#[test]
fn redis_precedence_ignores_the_environment_url_once_a_host_is_configured() {
    let overrides = RedisOverrides {
        host: Some("configured.internal".to_owned()),
        ..blank_overrides()
    };
    let env = env(&[("REDIS_URL", "redis://from-environment:6379")]);
    let settings =
        resolve_redis_kwargs(overrides, ClientKind::Sync, &env).expect("host names a target");
    assert_eq!(settings.host, Some("configured.internal".to_owned()));
    assert!(settings.url.is_none());
}

#[test]
fn redis_precedence_falls_back_to_the_environment_url_without_an_explicit_target() {
    let env = env(&[("REDIS_URL", "redis://from-environment:6379")]);
    let settings = resolve_redis_kwargs(RedisOverrides::default(), ClientKind::Sync, &env)
        .expect("url names a target");
    assert_eq!(
        settings.url.as_ref().map(ExposeSecret::expose_secret),
        Some("redis://from-environment:6379")
    );
}

#[test]
fn redis_credential_provider_strips_userinfo_and_drops_username_and_password() {
    let overrides = RedisOverrides {
        url: Some(SecretString::from(
            "redis://user:pass@cache.internal:6379/0",
        )),
        credential_provider: true,
        ..blank_overrides()
    };
    let settings =
        resolve_redis_kwargs(overrides, ClientKind::Sync, &env(&[])).expect("url names a target");
    assert!(settings.username.is_none());
    assert!(settings.password.is_none());
    assert_eq!(
        settings.url.as_ref().map(ExposeSecret::expose_secret),
        Some("redis://cache.internal:6379/0")
    );
}

#[test]
fn redis_tls_is_true_end_to_end_for_a_rediss_url() {
    let overrides = RedisOverrides {
        url: Some(SecretString::from("rediss://cache.internal:6379")),
        ..blank_overrides()
    };
    let settings =
        resolve_redis_kwargs(overrides, ClientKind::Sync, &env(&[])).expect("url names a target");
    assert!(settings_uses_tls(&settings));
}

#[test]
fn redis_tls_is_false_end_to_end_for_a_plain_redis_url() {
    let overrides = RedisOverrides {
        url: Some(SecretString::from("redis://cache.internal:6379")),
        ..blank_overrides()
    };
    let settings =
        resolve_redis_kwargs(overrides, ClientKind::Sync, &env(&[])).expect("url names a target");
    assert!(!settings_uses_tls(&settings));
}

#[test]
fn redis_tls_uses_the_url_when_startup_nodes_are_empty() {
    let overrides = RedisOverrides {
        startup_nodes: Some(NodesInput::Nodes(Vec::new())),
        url: Some(SecretString::from("rediss://cache.internal:6379")),
        ..blank_overrides()
    };
    let settings =
        resolve_redis_kwargs(overrides, ClientKind::Sync, &env(&[])).expect("url names a target");
    assert!(settings_uses_tls(&settings));
}

#[test]
fn redis_tls_does_not_treat_an_empty_startup_list_as_cluster_mode() {
    let settings = RedisConnectionSettings {
        host: None,
        port: None,
        db: None,
        username: None,
        password: None,
        url: Some(SecretString::from("rediss://cache.internal:6379")),
        ssl: None,
        socket_timeout: None,
        socket_connect_timeout: None,
        socket_keepalive: None,
        max_connections: None,
        health_check_interval: None,
        credential_provider: false,
        startup_nodes: Some(Vec::new()),
        sentinel_nodes: None,
        sentinel_password: None,
        service_name: None,
        passthrough: BTreeMap::new(),
    };
    assert!(settings_uses_tls(&settings));
}

#[test]
fn redis_ssl_stays_unset_without_an_override_or_environment_value() {
    let overrides = RedisOverrides {
        host: Some("cache.internal".to_owned()),
        ..blank_overrides()
    };
    let settings =
        resolve_redis_kwargs(overrides, ClientKind::Sync, &env(&[])).expect("host names a target");
    assert_eq!(settings.ssl, None);
}

#[test]
fn redis_ssl_reads_false_from_the_environment() {
    let env = env(&[("REDIS_HOST", "cache.internal"), ("REDIS_SSL", "false")]);
    let settings = resolve_redis_kwargs(RedisOverrides::default(), ClientKind::Sync, &env)
        .expect("host names a target");
    assert_eq!(settings.ssl, Some(false));
}

#[test]
fn redis_precedence_ignores_the_environment_url_once_startup_nodes_are_configured() {
    let overrides = RedisOverrides {
        startup_nodes: Some(NodesInput::Nodes(vec![ClusterNodeConfig {
            host: "a".to_owned(),
            port: 7000,
        }])),
        ..blank_overrides()
    };
    let env = env(&[("REDIS_URL", "redis://from-environment:6379")]);
    let settings = resolve_redis_kwargs(overrides, ClientKind::Sync, &env)
        .expect("startup_nodes names a target");
    assert!(settings.url.is_none());
}

#[test]
fn redis_precedence_ignores_the_environment_url_for_an_empty_explicit_target() {
    let overrides = RedisOverrides {
        startup_nodes: Some(NodesInput::Nodes(Vec::new())),
        ..blank_overrides()
    };
    let env = env(&[("REDIS_URL", "redis://from-environment:6379")]);
    resolve_redis_kwargs(overrides, ClientKind::Sync, &env).unwrap_err();
}

#[test]
fn missing_target_is_an_error_end_to_end() {
    resolve_redis_kwargs(RedisOverrides::default(), ClientKind::Sync, &env(&[])).unwrap_err();
}

#[test]
fn cluster_startup_nodes_parse_from_environment_json() {
    let env = env(&[("REDIS_CLUSTER_NODES", r#"[{"host": "a", "port": 7000}]"#)]);
    let settings = resolve_redis_kwargs(RedisOverrides::default(), ClientKind::Sync, &env)
        .expect("startup_nodes names a target");
    assert_eq!(settings.startup_nodes.map(|nodes| nodes.len()), Some(1));
}

#[rstest]
fn replays_the_redis_settings_corpus(#[files("fuzz/corpus/redis_settings/*")] path: PathBuf) {
    let data = fs::read(path).unwrap();
    let Ok(overrides) = serde_json::from_slice::<RedisOverrides>(&data) else {
        return;
    };
    drop(resolve_redis_kwargs(overrides, ClientKind::Sync, &env(&[])));
}
