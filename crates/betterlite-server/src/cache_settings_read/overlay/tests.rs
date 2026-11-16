#![cfg(test)]

use betterlite_config::loader::startup::process_environment;
use betterlite_redis::redis_connection_settings::environment::EnvironmentLookup;
use rstest::rstest;
use serde_json::{Map, Value};

use super::{ENVIRONMENT_FIELDS, ProcessEnvironment, overlay_environment};
use crate::cache_settings_read::fields::catalog;
use crate::cache_settings_read::test_support::{environment, obj, parse};
use crate::cache_settings_read::view::{effective_values, fill_field_values};

fn overlay_with(stored: &Map<String, Value>, entries: &[(&str, &str)]) -> Value {
    Value::Object(overlay_environment(stored, &environment(entries)))
}

#[test]
fn the_overlay_reads_litellms_seventeen_redis_variables() {
    assert_eq!(
        ENVIRONMENT_FIELDS,
        [
            ("REDIS_AWS_IAM_AUTH", "aws_iam_auth"),
            ("REDIS_AWS_IAM_CACHE_NAME", "aws_iam_cache_name"),
            ("REDIS_AWS_IAM_REGION", "aws_iam_region"),
            ("REDIS_AWS_IAM_SERVERLESS", "aws_iam_serverless"),
            ("REDIS_AWS_IAM_USER_NAME", "aws_iam_user_name"),
            ("REDIS_DB", "db"),
            ("REDIS_GCP_SERVICE_ACCOUNT", "gcp_service_account"),
            ("REDIS_GCP_SSL_CA_CERTS", "gcp_ssl_ca_certs"),
            ("REDIS_HOST", "host"),
            ("REDIS_MAX_CONNECTIONS", "max_connections"),
            ("REDIS_PASSWORD", "password"),
            ("REDIS_PORT", "port"),
            ("REDIS_SSL", "ssl"),
            ("REDIS_SSL_CERT_REQS", "ssl_cert_reqs"),
            ("REDIS_SSL_CHECK_HOSTNAME", "ssl_check_hostname"),
            ("REDIS_URL", "url"),
            ("REDIS_USERNAME", "username"),
        ]
    );
}

#[test]
fn an_empty_store_and_environment_give_no_values_and_no_field_values() {
    let current_values = effective_values(&Map::new(), &environment(&[]));
    assert!(current_values.is_empty());
    let mut fields = catalog();
    fill_field_values(&mut fields, &current_values);
    let filled: Vec<&str> = fields
        .iter()
        .filter(|field| field.field_value.is_some())
        .map(|field| field.field_name)
        .collect();
    assert!(filled.is_empty(), "{filled:?}");
}

#[rstest]
#[case::aws_iam_auth("REDIS_AWS_IAM_AUTH", "aws_iam_auth")]
#[case::aws_iam_cache_name("REDIS_AWS_IAM_CACHE_NAME", "aws_iam_cache_name")]
#[case::aws_iam_region("REDIS_AWS_IAM_REGION", "aws_iam_region")]
#[case::aws_iam_serverless("REDIS_AWS_IAM_SERVERLESS", "aws_iam_serverless")]
#[case::aws_iam_user_name("REDIS_AWS_IAM_USER_NAME", "aws_iam_user_name")]
#[case::db("REDIS_DB", "db")]
#[case::gcp_service_account("REDIS_GCP_SERVICE_ACCOUNT", "gcp_service_account")]
#[case::gcp_ssl_ca_certs("REDIS_GCP_SSL_CA_CERTS", "gcp_ssl_ca_certs")]
#[case::host("REDIS_HOST", "host")]
#[case::max_connections("REDIS_MAX_CONNECTIONS", "max_connections")]
#[case::password("REDIS_PASSWORD", "password")]
#[case::port("REDIS_PORT", "port")]
#[case::ssl("REDIS_SSL", "ssl")]
#[case::ssl_cert_reqs("REDIS_SSL_CERT_REQS", "ssl_cert_reqs")]
#[case::ssl_check_hostname("REDIS_SSL_CHECK_HOSTNAME", "ssl_check_hostname")]
#[case::url("REDIS_URL", "url")]
#[case::username("REDIS_USERNAME", "username")]
fn every_redis_variable_fills_its_own_field(#[case] name: &str, #[case] field: &str) {
    let overlaid = overlay_with(&Map::new(), &[(name, "from-environment")]);
    let expected = parse(&format!(
        r#"{{"{field}": "from-environment", "type": "redis"}}"#
    ));
    assert_eq!(overlaid, expected, "{name}");
}

#[test]
fn a_redis_variable_outside_the_seventeen_names_changes_nothing() {
    let stored = obj(r#"{"host": "stored-host"}"#);
    let overlaid = overlay_with(
        &stored,
        &[
            ("REDIS_NAMESPACE", "unused"),
            ("REDIS_SENTINEL_NODES", "[['localhost', 26379]]"),
            ("REDIS_SENTINEL_PASSWORD", "unused"),
            ("REDIS_SERVICE_NAME", "unused"),
            ("REDIS_SOCKET_TIMEOUT", "5"),
            ("REDIS_TTL", "60"),
        ],
    );
    assert_eq!(overlaid, Value::Object(stored));
}

#[test]
fn a_stored_key_wins_over_the_environment_even_when_it_holds_null() {
    let stored = obj(r#"{"host": null, "port": "6380"}"#);
    let overlaid = overlay_with(
        &stored,
        &[
            ("REDIS_HOST", "environment-host"),
            ("REDIS_PORT", "6379"),
            ("REDIS_DB", "2"),
        ],
    );
    assert_eq!(
        overlaid,
        parse(r#"{"host": null, "port": "6380", "db": "2", "type": "redis"}"#)
    );
}

#[test]
fn a_stored_type_survives_the_environment_overlay() {
    let stored = obj(r#"{"type": "local"}"#);
    let overlaid = overlay_with(&stored, &[("REDIS_HOST", "environment-host")]);
    assert_eq!(
        overlaid,
        parse(r#"{"type": "local", "host": "environment-host"}"#)
    );
}

#[test]
fn boolean_fields_read_the_shared_bool_words_and_other_fields_stay_strings() {
    let overlaid = overlay_with(
        &Map::new(),
        &[
            ("REDIS_SSL", " TRUE "),
            ("REDIS_SSL_CHECK_HOSTNAME", "no"),
            ("REDIS_AWS_IAM_AUTH", "1"),
            ("REDIS_AWS_IAM_SERVERLESS", "off"),
            ("REDIS_PORT", " 6379 "),
            ("REDIS_HOST", "true"),
        ],
    );
    assert_eq!(
        overlaid,
        parse(
            r#"{"ssl": true, "ssl_check_hostname": false, "aws_iam_auth": true, "aws_iam_serverless": false, "port": " 6379 ", "host": "true", "type": "redis"}"#
        )
    );
}

#[test]
fn a_boolean_field_with_other_text_keeps_the_raw_string() {
    assert_eq!(
        overlay_with(&Map::new(), &[("REDIS_SSL", "maybe")]),
        parse(r#"{"ssl": "maybe", "type": "redis"}"#)
    );
}

#[rstest]
#[case::empty("")]
#[case::blank("  ")]
fn an_empty_environment_value_counts_as_unset(#[case] value: &str) {
    assert_eq!(
        overlay_with(&Map::new(), &[("REDIS_HOST", value), ("REDIS_SSL", value)]),
        parse("{}")
    );
}

#[test]
fn type_defaults_to_redis_only_after_a_redis_variable_was_found() {
    assert_eq!(overlay_with(&Map::new(), &[]), parse("{}"));
    assert_eq!(
        overlay_with(&Map::new(), &[("REDIS_HOST", "environment-host")]),
        parse(r#"{"host": "environment-host", "type": "redis"}"#)
    );
}

#[test]
fn the_process_environment_lookup_mirrors_the_process_environment() {
    let snapshot = process_environment();
    let Some((name, value)) = snapshot.iter().next() else {
        panic!("nextest gives the test process an environment");
    };
    assert_eq!(ProcessEnvironment.lookup(name), Some(value.clone()));
}

#[test]
fn a_name_the_process_does_not_hold_is_absent() {
    assert_eq!(ProcessEnvironment.lookup("REDIS_B4307_TEST_NAME"), None);
}
