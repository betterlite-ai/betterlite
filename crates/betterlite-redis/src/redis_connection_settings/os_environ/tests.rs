#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::{ExposeSecret, SecretString};

use super::resolve_overrides_os_environ;
use crate::redis_connection_settings::model::scalars::{FloatOrText, IntOrText, PassthroughValue};
use crate::redis_connection_settings::model::settings::RedisOverrides;

fn env_with(name: &str, value: &str) -> BTreeMap<String, String> {
    BTreeMap::from([(name.to_owned(), value.to_owned())])
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
        ssl_password: None,
        gcp_service_account: None,
        gcp_ssl_ca_certs: None,
        azure_redis_ad_token: None,
        azure_client_id: None,
        azure_tenant_id: None,
        azure_client_secret: None,
        aws_iam_auth: None,
        aws_iam_user_name: None,
        aws_iam_cache_name: None,
        aws_iam_region: None,
        aws_iam_serverless: None,
        passthrough: BTreeMap::new(),
    }
}

#[test]
fn an_os_environ_reference_resolves_before_any_other_field_is_touched() {
    let overrides = RedisOverrides {
        host: Some("os.environ/MY_REDIS_HOST".to_owned()),
        ..blank_overrides()
    };
    let env = env_with("MY_REDIS_HOST", "cache.internal");

    let resolved = resolve_overrides_os_environ(overrides, &env);
    assert_eq!(resolved.host, Some("cache.internal".to_owned()));
}

#[test]
fn a_plain_string_is_left_untouched() {
    let overrides = RedisOverrides {
        host: Some("cache.internal".to_owned()),
        ..blank_overrides()
    };

    let resolved = resolve_overrides_os_environ(overrides, &BTreeMap::new());
    assert_eq!(resolved.host, Some("cache.internal".to_owned()));
}

#[test]
fn an_unresolved_reference_clears_the_field() {
    let overrides = RedisOverrides {
        host: Some("os.environ/MISSING_VAR".to_owned()),
        ..blank_overrides()
    };

    let resolved = resolve_overrides_os_environ(overrides, &BTreeMap::new());
    assert!(resolved.host.is_none());
}

#[test]
fn a_secret_field_resolves_through_the_reference_too() {
    let overrides = RedisOverrides {
        password: Some(SecretString::from(
            "os.environ/MY_REDIS_PASSWORD".to_owned(),
        )),
        ..blank_overrides()
    };
    let env = env_with("MY_REDIS_PASSWORD", "s3cret");

    let resolved = resolve_overrides_os_environ(overrides, &env);
    assert_eq!(
        resolved.password.as_ref().map(ExposeSecret::expose_secret),
        Some("s3cret")
    );
}

#[test]
fn an_explicit_null_url_stays_null_after_resolution() {
    let overrides = RedisOverrides {
        url: Some(None),
        ..blank_overrides()
    };

    let resolved = resolve_overrides_os_environ(overrides, &BTreeMap::new());
    assert!(matches!(resolved.url, Some(None)));
}

#[test]
fn a_present_url_resolves_through_the_reference() {
    let overrides = RedisOverrides {
        url: Some(Some(SecretString::from(
            "os.environ/MY_REDIS_URL".to_owned(),
        ))),
        ..blank_overrides()
    };
    let env = env_with("MY_REDIS_URL", "redis://cache:6379");

    let resolved = resolve_overrides_os_environ(overrides, &env);
    assert_eq!(
        resolved
            .url
            .flatten()
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("redis://cache:6379")
    );
}

#[test]
fn an_int_field_resolves_through_the_reference() {
    let overrides = RedisOverrides {
        port: Some(IntOrText::Text("os.environ/MY_REDIS_PORT".to_owned())),
        ..blank_overrides()
    };
    let env = env_with("MY_REDIS_PORT", "6380");

    let resolved = resolve_overrides_os_environ(overrides, &env);
    assert_eq!(resolved.port, Some(IntOrText::Text("6380".to_owned())));
}

#[test]
fn a_float_field_resolves_through_the_reference() {
    let overrides = RedisOverrides {
        socket_timeout: Some(FloatOrText::Text("os.environ/MY_SOCKET_TIMEOUT".to_owned())),
        ..blank_overrides()
    };
    let env = env_with("MY_SOCKET_TIMEOUT", "2.5");

    let resolved = resolve_overrides_os_environ(overrides, &env);
    assert_eq!(
        resolved.socket_timeout,
        Some(FloatOrText::Text("2.5".to_owned()))
    );
}

#[test]
fn a_real_int_is_left_untouched() {
    let overrides = RedisOverrides {
        port: Some(IntOrText::Int(6380)),
        ..blank_overrides()
    };

    let resolved = resolve_overrides_os_environ(overrides, &BTreeMap::new());
    assert_eq!(resolved.port, Some(IntOrText::Int(6380)));
}

#[test]
fn a_passthrough_value_resolves_through_the_reference() {
    let mut overrides = RedisOverrides::default();
    overrides.passthrough.insert(
        "client_name".to_owned(),
        PassthroughValue::Text("os.environ/MY_CLIENT_NAME".to_owned()),
    );
    let env = env_with("MY_CLIENT_NAME", "betterlite");

    let resolved = resolve_overrides_os_environ(overrides, &env);
    assert_eq!(
        resolved.passthrough.get("client_name"),
        Some(&PassthroughValue::Text("betterlite".to_owned()))
    );
}

#[test]
fn an_unresolved_passthrough_reference_drops_the_key() {
    let mut overrides = RedisOverrides::default();
    overrides.passthrough.insert(
        "client_name".to_owned(),
        PassthroughValue::Text("os.environ/MISSING_VAR".to_owned()),
    );

    let resolved = resolve_overrides_os_environ(overrides, &BTreeMap::new());
    assert!(!resolved.passthrough.contains_key("client_name"));
}
