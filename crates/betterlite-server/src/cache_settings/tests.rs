#![cfg(test)]

use std::fs;
use std::path::Path;

use rstest::rstest;
use serde_json::{Map, Value};

use super::REDACTED_VALUE;
use super::merge::merge_over_saved;
use super::parse::{ParseStoredSettingsError, parse_stored_settings};
use super::precedence::resolve_cache_url_precedence;
use super::redact::{is_credential_field, redact_credentials, redact_settings};
use super::reuse::saved_secret_is_reusable;
use super::target::{has_connection_target, target_repr};

fn parse(text: &str) -> Value {
    serde_json::from_str(text).expect("valid JSON fixture")
}

fn obj(text: &str) -> Map<String, Value> {
    parse(text).as_object().expect("object fixture").clone()
}

fn str_value(text: &str) -> Value {
    Value::String(text.to_owned())
}

fn marker_field(key: &str) -> Map<String, Value> {
    let mut map = Map::new();
    map.insert(key.to_owned(), str_value(REDACTED_VALUE));
    map
}

#[rstest]
#[case::url_supplied(
    obj(r#"{"url": "redis://host:6379", "host": "ignored", "port": 1, "db": 0, "password": "p", "username": "u"}"#),
    parse(r#"{"url": "redis://host:6379"}"#)
)]
#[case::no_url(
    obj(r#"{"host": "h", "port": 1}"#),
    parse(r#"{"host": "h", "port": 1}"#)
)]
#[case::cluster_mode_keeps_both(
    obj(r#"{"url": "redis://host:6379", "redis_startup_nodes": [{"host": "a", "port": 1}]}"#),
    parse(r#"{"url": "redis://host:6379", "redis_startup_nodes": [{"host": "a", "port": 1}]}"#)
)]
#[case::blank_url_is_not_a_url(
    obj(r#"{"url": "  ", "host": "h"}"#),
    parse(r#"{"url": "  ", "host": "h"}"#)
)]
#[case::numeric_cluster_nodes_counts_as_cluster_mode(
    obj(r#"{"url": "redis://host:6379", "redis_startup_nodes": 0, "host": "ignored"}"#),
    parse(r#"{"url": "redis://host:6379", "redis_startup_nodes": 0, "host": "ignored"}"#)
)]
fn resolve_cache_url_precedence_behavior(
    #[case] input: Map<String, Value>,
    #[case] expected: Value,
) {
    assert_eq!(
        Value::Object(resolve_cache_url_precedence(&input)),
        expected
    );
}

#[rstest]
#[case::string_form(
    str_value(r#"{"host": "h", "port": 1}"#),
    parse(r#"{"host": "h", "port": 1}"#)
)]
#[case::dict_form(parse(r#"{"host": "h"}"#), parse(r#"{"host": "h"}"#))]
#[case::non_dict_parsed(str_value("[1,2,3]"), parse("{}"))]
fn parse_stored_settings_normal_forms(#[case] input: Value, #[case] expected: Value) {
    let parsed = parse_stored_settings(&input).expect("parses");
    assert_eq!(Value::Object(parsed), expected);
}

#[test]
fn parse_stored_settings_rejects_invalid_json_text() {
    let error = parse_stored_settings(&str_value("{not valid")).unwrap_err();
    assert!(matches!(
        error,
        ParseStoredSettingsError::InvalidJson { .. }
    ));
}

#[rstest]
#[case::explicit_password("password", true)]
#[case::explicit_sentinel_password("sentinel_password", true)]
#[case::explicit_url("url", true)]
#[case::classifier_detected("api_key", true)]
#[case::ordinary_field("host", false)]
fn is_credential_field_behavior(#[case] key: &str, #[case] expected: bool) {
    assert_eq!(is_credential_field(key), expected);
}

#[test]
fn redact_credentials_masks_only_credential_fields() {
    let settings = obj(r#"{"password": "secret", "host": "h", "api_key": "k", "port": null}"#);
    let redacted = redact_credentials(&settings);
    assert_eq!(redacted["password"], REDACTED_VALUE);
    assert_eq!(redacted["api_key"], REDACTED_VALUE);
    assert_eq!(redacted["host"], "h");
    assert_eq!(redacted["port"], Value::Null);
}

#[rstest]
#[case::non_empty_string(str_value("host"), true)]
#[case::empty_string(str_value(""), false)]
#[case::blank_string(str_value("   "), false)]
#[case::none(Value::Null, false)]
#[case::marker(str_value(REDACTED_VALUE), false)]
#[case::non_empty_list(parse("[1]"), true)]
#[case::empty_list(parse("[]"), false)]
#[case::non_empty_dict(parse(r#"{"a": 1}"#), true)]
#[case::empty_dict(parse("{}"), false)]
#[case::number(parse("0"), true)]
fn has_connection_target_behavior(#[case] value: Value, #[case] expected: bool) {
    assert_eq!(has_connection_target(&value), expected);
}

#[rstest]
#[case::string(str_value("redis-1"), "redis-1")]
#[case::int_port(parse("6379"), "6379")]
#[case::none(Value::Null, "None")]
#[case::bool_true(Value::Bool(true), "True")]
#[case::bool_false(Value::Bool(false), "False")]
#[case::list_normalized(
    parse(r#"[{"port": 1}, {"port": 2}]"#),
    r#"[{"port": 1}, {"port": 2}]"#
)]
#[case::dict_sorted(
    parse(r#"{"port": 6379, "host": "a"}"#),
    r#"{"host": "a", "port": 6379}"#
)]
fn target_repr_behavior(#[case] value: Value, #[case] expected: &str) {
    assert_eq!(target_repr(&value), expected);
}

#[rstest]
#[case::same_host(
    obj(r#"{"host": "a", "port": 1}"#),
    obj(r#"{"host": "a", "port": 1}"#),
    true
)]
#[case::different_host(
    obj(r#"{"host": "b", "port": 1}"#),
    obj(r#"{"host": "a", "port": 1}"#),
    false
)]
#[case::url_omitted_keeps_saved(Map::new(), obj(r#"{"url": "redis://host"}"#), true)]
#[case::url_redacted_keeps_saved(marker_field("url"), obj(r#"{"url": "redis://host"}"#), true)]
#[case::missing_incoming_field_breaks_match(Map::new(), obj(r#"{"host": "a"}"#), false)]
fn saved_secret_is_reusable_behavior(
    #[case] incoming: Map<String, Value>,
    #[case] saved: Map<String, Value>,
    #[case] expected: bool,
) {
    assert_eq!(saved_secret_is_reusable(&incoming, &saved), expected);
}

#[test]
fn merge_over_saved_restores_omitted_password() {
    let incoming = obj(r#"{"host": "a"}"#);
    let saved = obj(r#"{"host": "a", "password": "secret"}"#);
    let merged = merge_over_saved(&incoming, &saved);
    assert_eq!(merged["password"], "secret");
}

#[test]
fn merge_over_saved_restores_redacted_marker() {
    let mut incoming = obj(r#"{"host": "a"}"#);
    incoming.insert("password".to_owned(), str_value(REDACTED_VALUE));
    let saved = obj(r#"{"host": "a", "password": "secret"}"#);
    let merged = merge_over_saved(&incoming, &saved);
    assert_eq!(merged["password"], "secret");
}

#[test]
fn merge_over_saved_keeps_explicit_new_value() {
    let incoming = obj(r#"{"host": "a", "password": "new-secret"}"#);
    let saved = obj(r#"{"host": "a", "password": "old-secret"}"#);
    let merged = merge_over_saved(&incoming, &saved);
    assert_eq!(merged["password"], "new-secret");
}

#[test]
fn merge_over_saved_drops_url_when_switching_to_discrete_target() {
    let incoming = obj(r#"{"host": "a"}"#);
    let saved = obj(r#"{"url": "redis://host"}"#);
    let merged = merge_over_saved(&incoming, &saved);
    assert!(!merged.contains_key("url"));
}

#[test]
fn merge_over_saved_drops_marker_when_nothing_is_stored() {
    let incoming = marker_field("password");
    let saved = Map::new();
    let merged = merge_over_saved(&incoming, &saved);
    assert!(!merged.contains_key("password"));
}

#[test]
fn redact_settings_masks_every_value() {
    let settings = obj(r#"{"host": "a", "password": "secret"}"#);
    let redacted = redact_settings(Some(&settings));
    assert_eq!(redacted["host"], REDACTED_VALUE);
    assert_eq!(redacted["password"], REDACTED_VALUE);
}

#[test]
fn redact_settings_handles_absent_and_empty() {
    assert_eq!(redact_settings(None), Map::new());
    assert_eq!(redact_settings(Some(&Map::new())), Map::new());
}

fn replay_corpus() -> usize {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/cache_settings_parser");
    let mut count = 0_usize;
    for entry in fs::read_dir(&root).unwrap() {
        let bytes = fs::read(entry.unwrap().path()).unwrap();
        if let Ok(text) = String::from_utf8(bytes) {
            drop(parse_stored_settings(&str_value(&text)));
        }
        count = count.saturating_add(1);
    }
    count
}

#[test]
fn replays_the_cache_settings_secrets_corpus() {
    assert!(replay_corpus() > 0);
}
