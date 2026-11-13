use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::SecretString;

use super::fixtures::empty_settings;
use crate::redis_client_reuse::cache_key::async_client_cache_key;
use crate::redis_connection_settings::model::nodes::ClusterNodeConfig;
use crate::redis_connection_settings::model::scalars::PassthroughValue;
use crate::redis_connection_settings::model::settings::{
    CredentialProviderSettings, RedisConnectionSettings,
};

fn base() -> RedisConnectionSettings {
    let mut settings = empty_settings();
    settings.host = Some("cache.internal".to_owned());
    settings.port = Some(6379);
    settings.password = Some(SecretString::from("base-secret".to_owned()));
    settings
}

fn set_host(settings: &mut RedisConnectionSettings) {
    settings.host = Some("other.internal".to_owned());
}

fn set_port(settings: &mut RedisConnectionSettings) {
    settings.port = Some(6380);
}

fn set_db(settings: &mut RedisConnectionSettings) {
    settings.db = Some(1);
}

fn set_username(settings: &mut RedisConnectionSettings) {
    settings.username = Some("other-user".to_owned());
}

fn set_password(settings: &mut RedisConnectionSettings) {
    settings.password = Some(SecretString::from("other-secret".to_owned()));
}

fn set_url(settings: &mut RedisConnectionSettings) {
    settings.url = Some(SecretString::from("redis://other.internal:6379"));
}

fn set_ssl(settings: &mut RedisConnectionSettings) {
    settings.ssl = Some(true);
}

fn set_socket_timeout(settings: &mut RedisConnectionSettings) {
    settings.socket_timeout = Some(2.5_f64);
}

fn set_socket_connect_timeout(settings: &mut RedisConnectionSettings) {
    settings.socket_connect_timeout = Some(2.5_f64);
}

fn set_socket_keepalive(settings: &mut RedisConnectionSettings) {
    settings.socket_keepalive = Some(true);
}

fn set_max_connections(settings: &mut RedisConnectionSettings) {
    settings.max_connections = Some(10);
}

fn set_health_check_interval(settings: &mut RedisConnectionSettings) {
    settings.health_check_interval = Some(30);
}

fn set_credential_provider(settings: &mut RedisConnectionSettings) {
    settings.credential_provider = true;
}

fn set_credential_provider_settings(settings: &mut RedisConnectionSettings) {
    settings.credential_provider_settings = Some(CredentialProviderSettings::Gcp {
        service_account: "service@example.iam.gserviceaccount.com".to_owned(),
        ssl_ca_certs: None,
    });
}

fn set_startup_nodes(settings: &mut RedisConnectionSettings) {
    settings.startup_nodes = Some(vec![ClusterNodeConfig {
        host: "node.internal".to_owned(),
        port: 7000,
    }]);
}

fn set_sentinel_nodes(settings: &mut RedisConnectionSettings) {
    settings.sentinel_nodes = Some(vec![("sentinel.internal".to_owned(), 26379)]);
}

fn set_sentinel_password(settings: &mut RedisConnectionSettings) {
    settings.sentinel_password = Some(SecretString::from("sentinel-secret".to_owned()));
}

fn set_service_name(settings: &mut RedisConnectionSettings) {
    settings.service_name = Some("primary".to_owned());
}

fn set_ssl_password(settings: &mut RedisConnectionSettings) {
    settings.ssl_password = Some(SecretString::from("ssl-secret".to_owned()));
}

fn set_passthrough(settings: &mut RedisConnectionSettings) {
    settings
        .passthrough
        .insert("retry_on_timeout".to_owned(), PassthroughValue::Bool(true));
}

#[test]
fn the_key_has_the_litellm_prefix_and_sixteen_hex_characters() {
    let key = async_client_cache_key(&base());
    let hash = key
        .strip_prefix("async-redis-client-")
        .expect("the litellm cache key prefix");
    assert_eq!(hash.chars().count(), 16);
    assert!(hash.chars().all(|character| character.is_ascii_hexdigit()));
    assert!(!hash.chars().any(|character| character.is_ascii_uppercase()));
}

#[test]
fn equal_settings_give_the_same_key() {
    assert_eq!(
        async_client_cache_key(&base()),
        async_client_cache_key(&base())
    );
}

#[test]
fn reordering_passthrough_entries_keeps_the_key() {
    let mut left = base();
    let mut left_entries = BTreeMap::new();
    left_entries.insert("decode_responses".to_owned(), PassthroughValue::Bool(true));
    left_entries.insert("retry_on_timeout".to_owned(), PassthroughValue::Bool(true));
    left.passthrough = left_entries;
    let mut right = base();
    let mut right_entries = BTreeMap::new();
    right_entries.insert("retry_on_timeout".to_owned(), PassthroughValue::Bool(true));
    right_entries.insert("decode_responses".to_owned(), PassthroughValue::Bool(true));
    right.passthrough = right_entries;
    assert_eq!(
        async_client_cache_key(&left),
        async_client_cache_key(&right)
    );
}

#[test]
fn the_key_never_contains_the_password() {
    let key = async_client_cache_key(&base());
    assert!(!key.contains("base-secret"));
    assert!(!format!("{key:?}").contains("base-secret"));
}

#[rstest]
#[case::host(set_host)]
#[case::port(set_port)]
#[case::db(set_db)]
#[case::username(set_username)]
#[case::password(set_password)]
#[case::url(set_url)]
#[case::ssl(set_ssl)]
#[case::socket_timeout(set_socket_timeout)]
#[case::socket_connect_timeout(set_socket_connect_timeout)]
#[case::socket_keepalive(set_socket_keepalive)]
#[case::max_connections(set_max_connections)]
#[case::health_check_interval(set_health_check_interval)]
#[case::credential_provider(set_credential_provider)]
#[case::credential_provider_settings(set_credential_provider_settings)]
#[case::startup_nodes(set_startup_nodes)]
#[case::sentinel_nodes(set_sentinel_nodes)]
#[case::sentinel_password(set_sentinel_password)]
#[case::service_name(set_service_name)]
#[case::ssl_password(set_ssl_password)]
#[case::passthrough(set_passthrough)]
fn every_field_changes_the_key(#[case] mutate: fn(&mut RedisConnectionSettings)) {
    let mut settings = base();
    let original = async_client_cache_key(&settings);
    mutate(&mut settings);
    assert_ne!(async_client_cache_key(&settings), original);
}
