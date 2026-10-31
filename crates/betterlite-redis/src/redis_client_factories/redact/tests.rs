#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::SecretString;

use super::{connection_type_label, redacted_config_rows};
use crate::redis_connection_settings::model::nodes::ClusterNodeConfig;
use crate::redis_connection_settings::model::scalars::PassthroughValue;
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

fn row_value(rows: &[super::ConfigRow], key: &str) -> Option<String> {
    rows.iter()
        .find(|row| row.key == key)
        .map(|row| row.value.clone())
}

#[test]
fn redaction_substitutes_a_placeholder_for_a_set_credential_provider() {
    let settings = RedisConnectionSettings {
        credential_provider: true,
        credential_provider_settings: Some(CredentialProviderSettings::Explicit),
        ..blank_settings()
    };
    let rows = redacted_config_rows(&settings);
    let value = row_value(&rows, "credential_provider").expect("credential_provider row exists");
    assert!(value.starts_with("<cre"));
    assert!(!value.contains("Explicit"));
    assert!(!value.contains("provider>"));
}

#[test]
fn redaction_omits_credential_provider_when_it_was_never_set() {
    let rows = redacted_config_rows(&blank_settings());
    assert!(row_value(&rows, "credential_provider").is_none());
}

#[test]
fn redaction_substitutes_a_placeholder_for_a_set_redis_connect_func() {
    let mut settings = blank_settings();
    settings.passthrough.insert(
        "redis_connect_func".to_owned(),
        PassthroughValue::Text("iam-callback".to_owned()),
    );
    let rows = redacted_config_rows(&settings);
    let value = row_value(&rows, "redis_connect_func").expect("redis_connect_func row exists");
    assert!(value.contains("redis connect function"));
    assert!(!value.contains("iam-callback"));
}

#[test]
fn redaction_masks_the_data_node_password() {
    let settings = RedisConnectionSettings {
        password: Some(SecretString::from("super-secret-value".to_owned())),
        ..blank_settings()
    };
    let rows = redacted_config_rows(&settings);
    let value = row_value(&rows, "password").expect("password row exists");
    assert!(!value.contains("super-secret-value"));
}

#[test]
fn redaction_masks_the_sentinel_password_too() {
    let settings = RedisConnectionSettings {
        sentinel_password: Some(SecretString::from("monitor-only-secret".to_owned())),
        ..blank_settings()
    };
    let rows = redacted_config_rows(&settings);
    let value = row_value(&rows, "sentinel_password").expect("sentinel_password row exists");
    assert!(!value.contains("monitor-only-secret"));
}

#[test]
fn redaction_leaves_a_non_sensitive_field_unmasked() {
    let settings = RedisConnectionSettings {
        host: Some("cache.internal".to_owned()),
        ..blank_settings()
    };
    let rows = redacted_config_rows(&settings);
    assert_eq!(row_value(&rows, "host"), Some("cache.internal".to_owned()));
}

#[test]
fn redaction_omits_fields_that_were_never_set() {
    let rows = redacted_config_rows(&blank_settings());
    assert!(row_value(&rows, "db").is_none());
    assert!(row_value(&rows, "service_name").is_none());
}

#[test]
fn redaction_names_the_literal_cluster_node_count() {
    let settings = RedisConnectionSettings {
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
        ..blank_settings()
    };
    let rows = redacted_config_rows(&settings);
    assert_eq!(
        row_value(&rows, "startup_nodes"),
        Some("[2 cluster nodes]".to_owned())
    );
}

#[test]
fn redaction_names_the_literal_sentinel_node_count() {
    let settings = RedisConnectionSettings {
        sentinel_nodes: Some(vec![
            ("sentinel-1".to_owned(), 26379),
            ("sentinel-2".to_owned(), 26379),
            ("sentinel-3".to_owned(), 26379),
        ]),
        ..blank_settings()
    };
    let rows = redacted_config_rows(&settings);
    assert_eq!(
        row_value(&rows, "sentinel_nodes"),
        Some("[3 sentinel nodes]".to_owned())
    );
}

#[test]
fn connection_type_follows_cluster_sentinel_url_direct_priority() {
    let cluster = RedisConnectionSettings {
        startup_nodes: Some(vec![ClusterNodeConfig {
            host: "node-a".to_owned(),
            port: 7000,
        }]),
        sentinel_nodes: Some(vec![("sentinel-1".to_owned(), 26379)]),
        url: Some(SecretString::from("redis://cache:6379".to_owned())),
        host: Some("cache".to_owned()),
        ..blank_settings()
    };
    assert_eq!(connection_type_label(&cluster), "Redis Cluster");

    let sentinel = RedisConnectionSettings {
        sentinel_nodes: Some(vec![("sentinel-1".to_owned(), 26379)]),
        url: Some(SecretString::from("redis://cache:6379".to_owned())),
        host: Some("cache".to_owned()),
        ..blank_settings()
    };
    assert_eq!(connection_type_label(&sentinel), "Redis Sentinel");

    let url = RedisConnectionSettings {
        url: Some(SecretString::from("redis://cache:6379".to_owned())),
        host: Some("cache".to_owned()),
        ..blank_settings()
    };
    assert_eq!(connection_type_label(&url), "Redis (URL-based)");

    let direct = RedisConnectionSettings {
        host: Some("cache".to_owned()),
        ..blank_settings()
    };
    assert_eq!(connection_type_label(&direct), "Standard Redis");
}
