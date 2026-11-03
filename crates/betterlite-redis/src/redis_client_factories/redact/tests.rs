#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
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
    assert_eq!(value, "******************");
}

#[test]
fn redaction_masks_the_sentinel_password_too() {
    let settings = RedisConnectionSettings {
        sentinel_password: Some(SecretString::from("monitor-only-secret".to_owned())),
        ..blank_settings()
    };
    let rows = redacted_config_rows(&settings);
    let value = row_value(&rows, "sentinel_password").expect("sentinel_password row exists");
    assert_eq!(value, "*******************");
}

#[rstest]
#[case::nine_chars("123456789")]
#[case::twelve_chars("abcdefghijkl")]
#[case::twenty_three_chars("12345678901234567890123")]
#[case::unicode_chars("p\u{e4}ssw\u{f6}rd-12")]
#[case::eight_chars("12345678")]
#[case::empty("")]
fn redaction_masks_a_secret_under_24_characters_fully(#[case] secret: &str) {
    let settings = RedisConnectionSettings {
        password: Some(SecretString::from(secret.to_owned())),
        sentinel_password: Some(SecretString::from(secret.to_owned())),
        ssl_password: Some(SecretString::from(secret.to_owned())),
        ..blank_settings()
    };
    let expected = Some("*".repeat(secret.chars().count()));
    let rows = redacted_config_rows(&settings);
    assert_eq!(row_value(&rows, "password"), expected);
    assert_eq!(row_value(&rows, "sentinel_password"), expected);
    assert_eq!(row_value(&rows, "ssl_password"), expected);
}

#[rstest]
#[case::twenty_four_chars("123456789012345678901234")]
#[case::thirty_chars("123456789012345678901234567890")]
fn redaction_keeps_four_and_four_of_a_secret_of_24_characters_or_more(#[case] secret: &str) {
    let settings = RedisConnectionSettings {
        password: Some(SecretString::from(secret.to_owned())),
        ..blank_settings()
    };
    let value = row_value(&redacted_config_rows(&settings), "password").expect("password row");
    let prefix: String = secret.chars().take(4).collect();
    let suffix: String = secret
        .chars()
        .rev()
        .take(4)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    assert_eq!(value.chars().take(4).collect::<String>(), prefix);
    assert!(value.ends_with(&suffix));
}

#[test]
fn redaction_keeps_litellms_mask_for_non_secret_rows() {
    let settings = RedisConnectionSettings {
        credential_provider_settings: Some(CredentialProviderSettings::Explicit),
        passthrough: BTreeMap::from([(
            "ssl_keyfile".to_owned(),
            PassthroughValue::Text("/etc/redis/client.key".to_owned()),
        )]),
        ..blank_settings()
    };
    let rows = redacted_config_rows(&settings);
    assert_eq!(
        row_value(&rows, "credential_provider"),
        Some("<cre*************der>".to_owned())
    );
    assert_eq!(
        row_value(&rows, "ssl_keyfile"),
        Some("/etc*************.key".to_owned())
    );
}

#[test]
fn redaction_strips_the_password_out_of_a_url() {
    let settings = RedisConnectionSettings {
        url: Some(SecretString::from(
            "redis://user:hunter2@cache.internal:6379/0".to_owned(),
        )),
        ..blank_settings()
    };
    let rows = redacted_config_rows(&settings);
    assert!(rows.iter().all(|row| !row.value.contains("hunter2")));
    let value = row_value(&rows, "url").expect("url row exists");
    assert!(!value.contains("user"));
}

#[test]
fn redaction_strips_a_password_carried_in_the_url_query_string() {
    let settings = RedisConnectionSettings {
        url: Some(SecretString::from(
            "redis://cache.internal:6379/0?password=hunter2".to_owned(),
        )),
        ..blank_settings()
    };
    let rows = redacted_config_rows(&settings);
    assert!(rows.iter().all(|row| !row.value.contains("hunter2")));
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
