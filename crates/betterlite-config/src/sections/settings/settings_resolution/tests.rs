#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::alerting::{EMAIL_DESCRIPTORS, MS_TEAMS_DESCRIPTORS, SLACK_DESCRIPTORS};
use super::{EmptyDbValue, FieldDescriptor, FieldSource, resolve_fields, source_for};
use crate::sections::settings::settings_rules::{DbRow, JsonValue, Section};
use crate::sections::settings::settings_store::SettingsStore;

const HOST: [FieldDescriptor; 1] = [FieldDescriptor {
    field_name: "host",
    db_key: "SMTP_HOST",
    env_var: "HOST_ENV",
    is_secret: false,
    default: None,
}];

fn json(text: &str) -> JsonValue {
    serde_json::from_str(text).unwrap()
}

fn stored(text: &str) -> BTreeMap<String, JsonValue> {
    serde_json::from_str(text).unwrap()
}

fn environment(value: Option<&str>) -> BTreeMap<String, SecretString> {
    value
        .map(|text| (String::from("HOST_ENV"), SecretString::from(text)))
        .into_iter()
        .collect()
}

fn host(row: &str, env: Option<&str>, empty: EmptyDbValue) -> (Option<String>, FieldSource) {
    let fields = resolve_fields(&HOST, &stored(row), &environment(env), empty);
    let field = fields.first().unwrap();
    assert_eq!(field.field_name, "host");
    (
        field
            .value
            .as_ref()
            .map(|value| value.expose_secret().to_owned()),
        field.source,
    )
}

#[rstest]
#[case::db_wins(r#"{"SMTP_HOST": "db"}"#, Some("env"), Some("db"), FieldSource::Db)]
#[case::empty_db_clears(r#"{"SMTP_HOST": ""}"#, Some("env"), Some(""), FieldSource::Db)]
#[case::blank_db_clears(r#"{"SMTP_HOST": "  "}"#, Some("env"), Some("  "), FieldSource::Db)]
#[case::null_db_falls_back(r#"{"SMTP_HOST": null}"#, Some("env"), Some("env"), FieldSource::Env)]
#[case::true_reads_like_python(r#"{"SMTP_HOST": true}"#, None, Some("True"), FieldSource::Db)]
#[case::false_reads_like_python(r#"{"SMTP_HOST": false}"#, None, Some("False"), FieldSource::Db)]
#[case::number_as_text(r#"{"SMTP_HOST": 2525}"#, None, Some("2525"), FieldSource::Db)]
#[case::list_as_json(
    r#"{"SMTP_HOST": [1, "a"]}"#,
    None,
    Some(r#"[1,"a"]"#),
    FieldSource::Db
)]
#[case::object_as_json(
    r#"{"SMTP_HOST": {"a": 1}}"#,
    None,
    Some(r#"{"a":1}"#),
    FieldSource::Db
)]
#[case::env_without_db("{}", Some(" env "), Some(" env "), FieldSource::Env)]
#[case::blank_env_is_unset("{}", Some(" \t\n"), None, FieldSource::Unset)]
#[case::separator_env_is_unset("{}", Some("\u{1c}\u{1f}"), None, FieldSource::Unset)]
#[case::nothing_is_unset("{}", None, None, FieldSource::Unset)]
fn empty_db_means_cleared_for_alerting(
    #[case] row: &str,
    #[case] env: Option<&str>,
    #[case] value: Option<&str>,
    #[case] source: FieldSource,
) {
    assert_eq!(
        host(row, env, EmptyDbValue::Cleared),
        (value.map(str::to_owned), source)
    );
}

#[rstest]
#[case::db_wins(r#"{"SMTP_HOST": "db"}"#, Some("env"), Some("db"))]
#[case::db_keeps_padding(r#"{"SMTP_HOST": " db "}"#, Some("env"), Some(" db "))]
#[case::empty_db_falls_back(r#"{"SMTP_HOST": ""}"#, Some("env"), Some("env"))]
#[case::blank_db_falls_back(r#"{"SMTP_HOST": " \u001c "}"#, Some("env"), Some("env"))]
#[case::null_db_falls_back(r#"{"SMTP_HOST": null}"#, Some("env"), Some("env"))]
#[case::bool_db_is_absent(r#"{"SMTP_HOST": true}"#, Some("env"), Some("env"))]
#[case::number_db_is_absent(r#"{"SMTP_HOST": 1}"#, None, None)]
fn empty_db_means_absent_for_sso(
    #[case] row: &str,
    #[case] env: Option<&str>,
    #[case] value: Option<&str>,
) {
    let fields = resolve_fields(&HOST, &stored(row), &environment(env), EmptyDbValue::Absent);
    let resolved: Vec<Option<&str>> = fields
        .iter()
        .map(|field| {
            field
                .value
                .as_ref()
                .map(secrecy::ExposeSecret::expose_secret)
        })
        .collect();
    assert_eq!(resolved, vec![value]);
}

#[rstest]
#[case::defaults("{}", &[], [("587", FieldSource::Default), ("True", FieldSource::Default)])]
#[case::stored_and_env(r#"{"SMTP_PORT": 25}"#, &[("SMTP_TLS", "False")], [("25", FieldSource::Db), ("False", FieldSource::Env)])]
fn email_port_and_tls_fall_back_to_mail_defaults(
    #[case] row: &str,
    #[case] env: &[(&str, &str)],
    #[case] expected: [(&str, FieldSource); 2],
) {
    let environment = env
        .iter()
        .map(|(name, value)| ((*name).to_owned(), SecretString::from(*value)))
        .collect();
    let fields = resolve_fields(
        &EMAIL_DESCRIPTORS,
        &stored(row),
        &environment,
        EmptyDbValue::Cleared,
    );
    let resolved: Vec<(Option<&str>, FieldSource)> = fields
        .iter()
        .map(|field| {
            (
                field
                    .value
                    .as_ref()
                    .map(secrecy::ExposeSecret::expose_secret),
                field.source,
            )
        })
        .collect();
    let unset = (None, FieldSource::Unset);
    let [port, tls] = expected.map(|(value, source)| (Some(value), source));
    assert_eq!(
        resolved,
        vec![unset, port, tls, unset, unset, unset, unset, unset, unset]
    );
}

#[rstest]
#[case::email(&EMAIL_DESCRIPTORS, &["SMTP_HOST", "SMTP_PORT", "SMTP_TLS", "SMTP_USERNAME", "SMTP_PASSWORD", "SMTP_SENDER_EMAIL", "TEST_EMAIL_ADDRESS", "EMAIL_LOGO_URL", "EMAIL_SUPPORT_CONTACT"], &["SMTP_USERNAME", "SMTP_PASSWORD"])]
#[case::slack(&SLACK_DESCRIPTORS, &["SLACK_WEBHOOK_URL"], &["SLACK_WEBHOOK_URL"])]
#[case::teams(&MS_TEAMS_DESCRIPTORS, &["MS_TEAMS_WEBHOOK_URL"], &["MS_TEAMS_WEBHOOK_URL"])]
fn alerting_tables_name_their_env_vars(
    #[case] descriptors: &[FieldDescriptor],
    #[case] names: &[&str],
    #[case] secrets: &[&str],
) {
    let listed: Vec<&str> = descriptors
        .iter()
        .map(|descriptor| descriptor.field_name)
        .collect();
    let secret: Vec<&str> = descriptors
        .iter()
        .filter(|descriptor| descriptor.is_secret)
        .map(|descriptor| descriptor.field_name)
        .collect();
    assert_eq!(listed, names);
    assert_eq!(secret, secrets);
    assert!(
        descriptors
            .iter()
            .all(|descriptor| descriptor.db_key == descriptor.field_name
                && descriptor.env_var == descriptor.field_name)
    );
}

#[rstest]
#[case::config_owned("owned", None, FieldSource::Config)]
#[case::config_owned_with_default("owned", Some("1"), FieldSource::Config)]
#[case::stored("stored", Some("1"), FieldSource::Db)]
#[case::unset_without_default("missing", None, FieldSource::Unset)]
#[case::unset_with_default("missing", Some("false"), FieldSource::Default)]
#[case::null_default_is_no_default("missing", Some("null"), FieldSource::Unset)]
fn source_for_reports_default_when_unset(
    #[case] key: &str,
    #[case] default: Option<&str>,
    #[case] expected: FieldSource,
) {
    let mut store = SettingsStore::from(Section::RouterSettings);
    store.load_yaml(stored(r#"{"owned": 1}"#));
    store.apply_db_row(
        DbRow::RouterSettings,
        stored(r#"{"stored": 2, "missing": null}"#),
    );
    assert_eq!(
        source_for(&store, key, &default.map_or(JsonValue::Null, json)),
        expected
    );
}

#[test]
fn resolved_secret_debug_output_is_redacted() {
    let descriptors = [FieldDescriptor {
        field_name: "secret",
        db_key: "SECRET",
        env_var: "SECRET",
        is_secret: true,
        default: None,
    }];
    let fields = resolve_fields(
        &descriptors,
        &BTreeMap::new(),
        &BTreeMap::from([(String::from("SECRET"), SecretString::from("password"))]),
        EmptyDbValue::Absent,
    );
    assert_eq!(
        fields[0].value.as_ref().unwrap().expose_secret(),
        "password"
    );
    assert!(!format!("{fields:?}").contains("password"));
}

#[test]
fn sources_serialize_as_litellm_names() {
    let sources = [
        FieldSource::Config,
        FieldSource::Db,
        FieldSource::Env,
        FieldSource::Default,
        FieldSource::Unset,
    ];
    assert_eq!(
        serde_json::to_string(&sources).unwrap(),
        r#"["config","db","env","default","unset"]"#
    );
}
