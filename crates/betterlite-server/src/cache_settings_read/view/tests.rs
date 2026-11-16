#![cfg(test)]

use rstest::rstest;
use serde_json::Value;

use super::{carries_value, effective_values, fill_field_values};
use crate::cache_settings::REDACTED_VALUE;
use crate::cache_settings_read::fields::catalog;
use crate::cache_settings_read::test_support::{environment, obj, parse};

fn view(stored: &str, entries: &[(&str, &str)]) -> Value {
    Value::Object(effective_values(&obj(stored), &environment(entries)))
}

#[test]
fn an_environment_url_drops_the_stored_discrete_fields() {
    let current_values = view(
        r#"{"host": "stored-host", "port": "6379", "db": 2, "password": "stored-secret", "username": "stored-user", "type": "redis"}"#,
        &[("REDIS_URL", "redis://:secret@env-host:6380/1")],
    );
    assert_eq!(
        current_values,
        parse(r#"{"url": "***REDACTED***", "type": "redis", "redis_type": "node"}"#)
    );
}

#[test]
fn a_stored_url_drops_the_environment_host_and_the_stored_db() {
    let current_values = view(
        r#"{"url": "redis://:secret@stored-host:6380/1", "db": 3, "type": "redis"}"#,
        &[("REDIS_HOST", "env-host"), ("REDIS_PORT", "6379")],
    );
    assert_eq!(
        current_values,
        parse(r#"{"url": "***REDACTED***", "type": "redis", "redis_type": "node"}"#)
    );
}

#[test]
fn cluster_mode_keeps_the_discrete_fields_a_url_would_drop() {
    let current_values = view(
        r#"{"url": "redis://:secret@cluster-host:6379/1", "host": "cluster-host", "port": "6379", "redis_startup_nodes": [{"host": "127.0.0.1", "port": 7001}], "type": "redis"}"#,
        &[],
    );
    assert_eq!(
        current_values,
        parse(
            r#"{"url": "***REDACTED***", "host": "cluster-host", "port": "6379", "redis_startup_nodes": [{"host": "127.0.0.1", "port": 7001}], "type": "redis", "redis_type": "cluster"}"#
        )
    );
}

#[rstest]
#[case::cluster(
    r#"{"type": "redis", "redis_startup_nodes": [{"host": "127.0.0.1", "port": 7001}], "sentinel_nodes": [["localhost", 26379]]}"#,
    "cluster"
)]
#[case::sentinel(
    r#"{"type": "redis", "sentinel_nodes": [["localhost", 26379]], "service_name": "mymaster"}"#,
    "sentinel"
)]
#[case::node(r#"{"type": "redis", "host": "host"}"#, "node")]
#[case::empty_sentinel_nodes(r#"{"type": "redis", "sentinel_nodes": []}"#, "node")]
#[case::false_sentinel_nodes(r#"{"type": "redis", "sentinel_nodes": false}"#, "node")]
#[case::empty_startup_nodes(r#"{"type": "redis", "redis_startup_nodes": []}"#, "node")]
#[case::zero_startup_nodes(r#"{"type": "redis", "redis_startup_nodes": 0}"#, "node")]
#[case::blank_startup_nodes(r#"{"type": "redis", "redis_startup_nodes": ""}"#, "node")]
#[case::null_startup_nodes(r#"{"type": "redis", "redis_startup_nodes": null}"#, "node")]
#[case::false_startup_nodes(r#"{"type": "redis", "redis_startup_nodes": false}"#, "node")]
fn redis_type_follows_cluster_then_sentinel_then_node(
    #[case] stored: &str,
    #[case] expected: &str,
) {
    let current_values = view(stored, &[]);
    assert_eq!(
        current_values.get("redis_type"),
        Some(&Value::String(expected.to_owned()))
    );
}

#[rstest]
#[case::local(r#"{"type": "local"}"#)]
#[case::null_type(r#"{"type": null}"#)]
#[case::other_spelling(r#"{"type": "Redis", "redis_startup_nodes": [["127.0.0.1", 7000]]}"#)]
fn a_type_that_is_not_redis_gets_no_redis_type(#[case] stored: &str) {
    let current_values = view(stored, &[]);
    assert_eq!(current_values.get("redis_type"), None);
}

#[test]
fn an_environment_only_connection_is_named_a_redis_node() {
    let current_values = view(
        r#"{}"#,
        &[("REDIS_HOST", "environment-host"), ("REDIS_PORT", "6379")],
    );
    assert_eq!(
        current_values,
        parse(
            r#"{"host": "environment-host", "port": "6379", "type": "redis", "redis_type": "node"}"#
        )
    );
}

#[test]
fn credentials_are_redacted_and_a_null_credential_stays_null() {
    let current_values = view(
        r#"{"type": "redis", "host": "host", "password": "secret", "sentinel_password": null, "aws_iam_auth": true}"#,
        &[],
    );
    assert_eq!(
        current_values,
        parse(
            r#"{"type": "redis", "host": "host", "password": "***REDACTED***", "sentinel_password": null, "aws_iam_auth": "***REDACTED***", "redis_type": "node"}"#
        )
    );
    assert_eq!(
        current_values.get("password"),
        Some(&Value::String(REDACTED_VALUE.to_owned()))
    );
    assert_eq!(current_values.get("sentinel_password"), Some(&Value::Null));
}

#[test]
fn a_redacted_url_drops_the_discrete_fields_it_replaces() {
    let current_values = view(
        r#"{"type": "redis", "url": "redis://:secret@host:6379/0", "host": "host", "password": "secret"}"#,
        &[],
    );
    assert_eq!(
        current_values,
        parse(r#"{"type": "redis", "url": "***REDACTED***", "redis_type": "node"}"#)
    );
}

#[test]
fn an_environment_credential_is_redacted_too() {
    let current_values = view(
        r#"{"host": "stored-host"}"#,
        &[("REDIS_PASSWORD", "environment-secret")],
    );
    assert_eq!(
        current_values,
        parse(
            r#"{"host": "stored-host", "password": "***REDACTED***", "type": "redis", "redis_type": "node"}"#
        )
    );
}

#[test]
fn field_values_come_from_the_redacted_current_values() {
    let current_values = effective_values(
        &obj(
            r#"{"type": "redis", "password": "secret", "sentinel_nodes": [["localhost", 26379]]}"#,
        ),
        &environment(&[]),
    );
    let mut fields = catalog();
    fill_field_values(&mut fields, &current_values);
    let value_of = |name: &str| {
        fields
            .iter()
            .find(|field| field.field_name == name)
            .expect("catalog field")
            .field_value
            .clone()
    };
    assert_eq!(
        value_of("password"),
        Some(Value::String(REDACTED_VALUE.to_owned()))
    );
    assert_eq!(
        value_of("redis_type"),
        Some(Value::String("sentinel".to_owned()))
    );
    assert_eq!(value_of("host"), None);
}

#[rstest]
#[case::null(Value::Null, false)]
#[case::false_flag(Value::Bool(false), false)]
#[case::true_flag(Value::Bool(true), true)]
#[case::zero(parse("0"), false)]
#[case::zero_float(parse("0.0"), false)]
#[case::negative(parse("-1"), true)]
#[case::empty_text(Value::String(String::new()), false)]
#[case::text(Value::String("x".to_owned()), true)]
#[case::empty_list(parse("[]"), false)]
#[case::list(parse("[1]"), true)]
#[case::empty_object(parse("{}"), false)]
#[case::object(parse(r#"{"a": 1}"#), true)]
fn python_truthiness_matches_litellm(#[case] value: Value, #[case] expected: bool) {
    assert_eq!(carries_value(&value), expected);
}
