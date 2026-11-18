#![cfg(test)]

use betterlite_redis::redis_client_reuse::transport::RedisCacheFamily;
use betterlite_redis::redis_connection_settings::model::nodes::ClusterNodeConfig;
use secrecy::ExposeSecret as _;

use super::support::{closed_port, environment, no_environment, params, wiring};
use crate::coord_redis_boot::build::{
    build_from_env, build_usage_redis, env_family, env_has_redis_target, usage_family,
};
use crate::coord_redis_boot::error::CoordRedisError;

fn environment_nodes(port: u16) -> String {
    format!(r#"[{{"host": "127.0.0.1", "port": {port}}}]"#)
}

#[test]
fn usage_family_prefers_params_nodes_over_environment() {
    let block = params(r#"{"startup_nodes": [{"host": "10.0.0.1", "port": 6379}]}"#);
    let env = environment(&[("REDIS_CLUSTER_NODES", &environment_nodes(6380))]);
    let family = usage_family(&block, &env).expect("the family resolves");
    assert_eq!(family, RedisCacheFamily::Cluster);
}

#[test]
fn usage_family_keeps_an_empty_params_list_plain() {
    let block = params(r#"{"host": "127.0.0.1", "startup_nodes": []}"#);
    let env = environment(&[("REDIS_CLUSTER_NODES", &environment_nodes(6380))]);
    let family = usage_family(&block, &env).expect("the family resolves");
    assert_eq!(family, RedisCacheFamily::Standalone);
}

#[test]
fn usage_family_reads_environment_nodes_when_params_have_none() {
    let block = params(r#"{"host": "127.0.0.1"}"#);
    let env = environment(&[("REDIS_CLUSTER_NODES", &environment_nodes(6380))]);
    let family = usage_family(&block, &env).expect("the family resolves");
    assert_eq!(family, RedisCacheFamily::Cluster);
}

#[test]
fn usage_family_keeps_an_empty_environment_list_plain() {
    let block = params(r#"{"host": "127.0.0.1"}"#);
    let empty = environment(&[("REDIS_CLUSTER_NODES", "[]")]);
    assert_eq!(
        usage_family(&block, &empty).expect("an empty list resolves"),
        RedisCacheFamily::Standalone
    );
    let null = environment(&[("REDIS_CLUSTER_NODES", "null")]);
    assert_eq!(
        usage_family(&block, &null).expect("a null resolves"),
        RedisCacheFamily::Standalone
    );
}

#[test]
fn usage_family_keeps_a_boolean_environment_text_plain() {
    let block = params(r#"{"host": "127.0.0.1"}"#);
    let env = environment(&[("REDIS_CLUSTER_NODES", "true")]);
    let family = usage_family(&block, &env).expect("a boolean text names no nodes");
    assert_eq!(family, RedisCacheFamily::Standalone);
}

#[test]
fn usage_family_keeps_a_false_json_value_plain() {
    let block = params(r#"{"host": "127.0.0.1"}"#);
    let zero = environment(&[("REDIS_CLUSTER_NODES", "0")]);
    assert_eq!(
        usage_family(&block, &zero).expect("zero is falsy"),
        RedisCacheFamily::Standalone
    );
    let object = environment(&[("REDIS_CLUSTER_NODES", "{}")]);
    assert_eq!(
        usage_family(&block, &object).expect("an empty object is falsy"),
        RedisCacheFamily::Standalone
    );
}

#[test]
fn usage_family_keeps_an_empty_environment_string_plain() {
    let block = params(r#"{"host": "127.0.0.1"}"#);
    let env = environment(&[("REDIS_CLUSTER_NODES", r#""""#)]);
    let family = usage_family(&block, &env).expect("an empty string is falsy");
    assert_eq!(family, RedisCacheFamily::Standalone);
}

#[test]
fn usage_family_rejects_a_malformed_environment_list() {
    let block = params(r#"{"host": "127.0.0.1"}"#);
    let env = environment(&[("REDIS_CLUSTER_NODES", "not json")]);
    let error = usage_family(&block, &env).expect_err("the list does not parse");
    assert!(matches!(error, CoordRedisError::ClusterNodes { .. }));
    assert!(
        error
            .to_string()
            .starts_with("REDIS_CLUSTER_NODES is not valid JSON: ")
    );
}

#[test]
fn env_has_redis_target_reads_host_and_url() {
    let host = environment(&[("REDIS_HOST", "redis.internal")]);
    assert!(env_has_redis_target(&host));
    let url = environment(&[("REDIS_URL", "redis://redis.internal:6379")]);
    assert!(env_has_redis_target(&url));
    let empty_host = environment(&[("REDIS_HOST", "")]);
    assert!(env_has_redis_target(&empty_host));
}

#[test]
fn env_has_redis_target_reads_the_node_lists() {
    let cluster = environment(&[("REDIS_CLUSTER_NODES", &environment_nodes(6379))]);
    assert!(env_has_redis_target(&cluster));
    let sentinel = environment(&[("REDIS_SENTINEL_NODES", r#"[["127.0.0.1", 26379]]"#)]);
    assert!(env_has_redis_target(&sentinel));
}

#[test]
fn env_has_redis_target_ignores_other_redis_variables() {
    let env = environment(&[
        ("REDIS_PORT", "6379"),
        ("REDIS_PASSWORD", "secret"),
        ("REDIS_DB", "1"),
        ("REDIS_USERNAME", "proxy"),
    ]);
    assert!(!env_has_redis_target(&env));
    assert!(!env_has_redis_target(&no_environment()));
}

#[test]
fn env_has_redis_target_ignores_boolean_node_texts() {
    let cluster = environment(&[("REDIS_CLUSTER_NODES", "TRUE")]);
    assert!(!env_has_redis_target(&cluster));
    let sentinel = environment(&[("REDIS_SENTINEL_NODES", "false")]);
    assert!(!env_has_redis_target(&sentinel));
}

#[tokio::test]
async fn build_usage_redis_builds_a_plain_client() {
    let port = closed_port();
    let block = params(&format!(r#"{{"host": "127.0.0.1", "port": {port}}}"#));
    let cache = build_usage_redis(&block, &wiring(), &no_environment())
        .await
        .expect("the plain client builds");
    assert_eq!(cache.transport.family, RedisCacheFamily::Standalone);
    assert_eq!(cache.transport.settings.host.as_deref(), Some("127.0.0.1"));
    assert_eq!(cache.transport.settings.port, Some(i64::from(port)));
}

#[tokio::test]
async fn build_usage_redis_builds_a_cluster_client() {
    let port = closed_port();
    let text = format!(r#"{{"startup_nodes": [{{"host": "127.0.0.1", "port": {port}}}]}}"#);
    let cache = build_usage_redis(&params(&text), &wiring(), &no_environment())
        .await
        .expect("the cluster client builds");
    assert_eq!(cache.transport.family, RedisCacheFamily::Cluster);
    assert_eq!(
        cache.transport.settings.startup_nodes,
        Some(vec![ClusterNodeConfig {
            host: "127.0.0.1".to_owned(),
            port: i64::from(port),
        }])
    );
}

#[tokio::test]
async fn build_usage_redis_takes_the_cluster_from_the_environment() {
    let port = closed_port();
    let block = params(&format!(r#"{{"host": "127.0.0.1", "port": {port}}}"#));
    let env = environment(&[("REDIS_CLUSTER_NODES", &environment_nodes(port))]);
    let cache = build_usage_redis(&block, &wiring(), &env)
        .await
        .expect("the cluster client builds from the environment nodes");
    assert_eq!(cache.transport.family, RedisCacheFamily::Cluster);
    assert_eq!(
        cache.transport.settings.startup_nodes,
        Some(vec![ClusterNodeConfig {
            host: "127.0.0.1".to_owned(),
            port: i64::from(port),
        }])
    );
}

#[tokio::test]
async fn build_usage_redis_reads_the_namespace_and_flush_size() {
    let port = closed_port();
    let text = format!(
        r#"{{"host": "127.0.0.1", "port": {port}, "namespace": "coordination", "redis_flush_size": 50}}"#
    );
    let cache = build_usage_redis(&params(&text), &wiring(), &no_environment())
        .await
        .expect("the cache-level options stay out of the client settings");
    assert_eq!(cache.namespace.as_deref(), Some("coordination"));
    assert_eq!(cache.transport.settings.host.as_deref(), Some("127.0.0.1"));
}

#[tokio::test]
async fn build_usage_redis_rejects_an_unknown_option() {
    let port = closed_port();
    let text = format!(r#"{{"host": "127.0.0.1", "port": {port}, "bogus_option": 1}}"#);
    let error = build_usage_redis(&params(&text), &wiring(), &no_environment())
        .await
        .err()
        .expect("a plain client has no such option");
    assert!(matches!(error, CoordRedisError::Build(_)));
}

#[tokio::test]
async fn build_usage_redis_drops_null_parameters() {
    let port = closed_port();
    let block = params(&format!(
        r#"{{"host": "127.0.0.1", "port": {port}, "unsupported_option": null}}"#
    ));
    let cache = build_usage_redis(&block, &wiring(), &no_environment())
        .await
        .expect("a null parameter never reaches the Redis client");
    assert_eq!(cache.transport.settings.port, Some(i64::from(port)));
}

#[tokio::test]
async fn build_from_env_returns_none_without_a_target() {
    let cache = build_from_env(&wiring(), &no_environment())
        .await
        .expect("no target is not an error");
    assert!(cache.is_none());
}

#[tokio::test]
async fn build_from_env_builds_from_the_environment() {
    let port = closed_port();
    let env = environment(&[
        ("REDIS_HOST", "127.0.0.1"),
        ("REDIS_PORT", &port.to_string()),
        ("REDIS_SOCKET_TIMEOUT", "1"),
        ("REDIS_SOCKET_CONNECT_TIMEOUT", "1"),
    ]);
    let cache = build_from_env(&wiring(), &env)
        .await
        .expect("the environment client builds")
        .expect("the environment names a target");
    assert_eq!(cache.transport.family, RedisCacheFamily::Standalone);
    assert_eq!(cache.transport.settings.host.as_deref(), Some("127.0.0.1"));
    assert_eq!(cache.transport.settings.port, Some(i64::from(port)));
}

#[tokio::test]
async fn build_from_env_honors_a_configured_socket_timeout() {
    let port = closed_port();
    let env = environment(&[
        ("REDIS_HOST", "127.0.0.1"),
        ("REDIS_PORT", &port.to_string()),
        ("REDIS_SOCKET_TIMEOUT", "1.5"),
    ]);
    let cache = build_from_env(&wiring(), &env)
        .await
        .expect("the environment client builds")
        .expect("the environment names a target");
    assert_eq!(cache.transport.settings.socket_timeout, Some(1.5_f64));
}

#[tokio::test]
async fn build_from_env_rejects_a_malformed_socket_timeout() {
    let port = closed_port();
    let env = environment(&[
        ("REDIS_HOST", "127.0.0.1"),
        ("REDIS_PORT", &port.to_string()),
        ("REDIS_SOCKET_TIMEOUT", "soon"),
    ]);
    let error = build_from_env(&wiring(), &env)
        .await
        .err()
        .expect("the timeout text does not parse");
    assert!(matches!(error, CoordRedisError::Build(_)));
    assert!(
        error.to_string().contains("REDIS_SOCKET_TIMEOUT"),
        "{error}"
    );
}

#[tokio::test]
async fn build_from_env_keeps_the_default_socket_timeout_when_unset() {
    let port = closed_port();
    let env = environment(&[
        ("REDIS_HOST", "127.0.0.1"),
        ("REDIS_PORT", &port.to_string()),
    ]);
    let cache = build_from_env(&wiring(), &env)
        .await
        .expect("the environment client builds")
        .expect("the environment names a target");
    assert_eq!(cache.transport.settings.socket_timeout, Some(5.0_f64));
}

#[tokio::test]
async fn build_from_env_keeps_the_default_socket_timeout_for_an_empty_value() {
    let port = closed_port();
    let env = environment(&[
        ("REDIS_HOST", "127.0.0.1"),
        ("REDIS_PORT", &port.to_string()),
        ("REDIS_SOCKET_TIMEOUT", ""),
    ]);
    let cache = build_from_env(&wiring(), &env)
        .await
        .expect("an empty timeout counts as unset")
        .expect("the environment names a target");
    assert_eq!(cache.transport.settings.socket_timeout, Some(5.0_f64));
}

#[tokio::test]
async fn build_usage_redis_keeps_the_credentials_secret() {
    let port = closed_port();
    let text = format!(
        r#"{{"host": "127.0.0.1", "port": {port}, "password": "s3cr3t", "sentinel_password": "sentinel"}}"#
    );
    let cache = build_usage_redis(&params(&text), &wiring(), &no_environment())
        .await
        .expect("the cache builds with its credentials");
    let password = cache
        .transport
        .settings
        .password
        .as_ref()
        .expect("the password reaches the client");
    assert_eq!(password.expose_secret(), "s3cr3t");
    let sentinel = cache
        .transport
        .settings
        .sentinel_password
        .as_ref()
        .expect("the sentinel password reaches the client");
    assert_eq!(sentinel.expose_secret(), "sentinel");
}

#[tokio::test]
async fn build_from_env_builds_a_cluster_from_environment_nodes() {
    let port = closed_port();
    let env = environment(&[("REDIS_CLUSTER_NODES", &environment_nodes(port))]);
    let cache = build_from_env(&wiring(), &env)
        .await
        .expect("the environment cluster builds")
        .expect("the node list names a target");
    assert_eq!(cache.transport.family, RedisCacheFamily::Cluster);
}

#[tokio::test]
async fn build_from_env_reports_a_malformed_node_list() {
    let env = environment(&[("REDIS_CLUSTER_NODES", "not json")]);
    let error = build_from_env(&wiring(), &env)
        .await
        .err()
        .expect("the node list does not parse");
    assert!(matches!(error, CoordRedisError::ClusterNodes { .. }));
}

#[test]
fn env_family_reads_the_environment_on_its_own() {
    let env = environment(&[("REDIS_CLUSTER_NODES", &environment_nodes(6379))]);
    assert_eq!(
        env_family(&env).expect("the environment nodes parse"),
        RedisCacheFamily::Cluster
    );
    assert_eq!(
        env_family(&no_environment()).expect("an empty environment is plain"),
        RedisCacheFamily::Standalone
    );
}
