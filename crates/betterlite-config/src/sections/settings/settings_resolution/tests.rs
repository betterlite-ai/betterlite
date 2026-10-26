#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::alerting::{EMAIL_DESCRIPTORS, MS_TEAMS_DESCRIPTORS, SLACK_DESCRIPTORS};
use super::{EmptyDbValue, FieldDescriptor, FieldSource, ResolvedField, resolve_fields, source_for};
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

fn environment(value: Option<&str>) -> BTreeMap<String, String> {
    value.map(|text| (String::from("HOST_ENV"), text.to_owned())).into_iter().collect()
}

fn host(row: &str, env: Option<&str>, empty: EmptyDbValue) -> (Option<String>, FieldSource) {
    let fields = resolve_fields(&HOST, &stored(row), &environment(env), empty).unwrap();
    let field = fields.first().unwrap();
    assert_eq!(field.field_name, "host");
    (field.value.clone(), field.source)
}

#[rstest]
#[case::db_wins(r#"{"SMTP_HOST": "db"}"#, Some("env"), Some("db"), FieldSource::Db)]
#[case::empty_db_clears(r#"{"SMTP_HOST": ""}"#, Some("env"), Some(""), FieldSource::Db)]
#[case::blank_db_clears(r#"{"SMTP_HOST": "  "}"#, Some("env"), Some("  "), FieldSource::Db)]
#[case::null_db_falls_back(r#"{"SMTP_HOST": null}"#, Some("env"), Some("env"), FieldSource::Env)]
#[case::true_reads_like_python(r#"{"SMTP_HOST": true}"#, None, Some("True"), FieldSource::Db)]
#[case::false_reads_like_python(r#"{"SMTP_HOST": false}"#, None, Some("False"), FieldSource::Db)]
#[case::number_as_text(r#"{"SMTP_HOST": 2525}"#, None, Some("2525"), FieldSource::Db)]
#[case::list_as_json(r#"{"SMTP_HOST": [1, "a"]}"#, None, Some(r#"[1,"a"]"#), FieldSource::Db)]
#[case::object_as_json(r#"{"SMTP_HOST": {"a": 1}}"#, None, Some(r#"{"a":1}"#), FieldSource::Db)]
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
    assert_eq!(host(row, env, EmptyDbValue::Cleared), (value.map(str::to_owned), source));
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
    let fields = resolve_fields(&HOST, &stored(row), &environment(env), EmptyDbValue::Absent).unwrap();
    let resolved: Vec<Option<&str>> = fields.iter().map(|field| field.value.as_deref()).collect();
    assert_eq!(resolved, vec![value]);
}

#[test]
fn email_defaults_fill_port_and_tls() {
    let fields = resolve_fields(&EMAIL_DESCRIPTORS, &BTreeMap::new(), &BTreeMap::new(), EmptyDbValue::Cleared).unwrap();
    let defaults: Vec<(&str, Option<String>, FieldSource)> = fields
        .into_iter()
        .map(|field| (field.field_name, field.value, field.source))
        .filter(|(_, _, source)| *source != FieldSource::Unset)
        .collect();
    assert_eq!(
        defaults,
        vec![
            ("SMTP_PORT", Some(String::from("587")), FieldSource::Default),
            ("SMTP_TLS", Some(String::from("True")), FieldSource::Default),
        ]
    );
}

#[test]
fn stored_and_env_values_beat_email_defaults() {
    let environment = BTreeMap::from([(String::from("SMTP_TLS"), String::from("False"))]);
    let fields = resolve_fields(&EMAIL_DESCRIPTORS, &stored(r#"{"SMTP_PORT": 25}"#), &environment, EmptyDbValue::Cleared).unwrap();
    assert_eq!(fields.get(1), Some(&ResolvedField { field_name: "SMTP_PORT", value: Some(String::from("25")), source: FieldSource::Db }));
    assert_eq!(fields.get(2), Some(&ResolvedField { field_name: "SMTP_TLS", value: Some(String::from("False")), source: FieldSource::Env }));
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
    let listed: Vec<&str> = descriptors.iter().map(|descriptor| descriptor.field_name).collect();
    let secret: Vec<&str> = descriptors.iter().filter(|descriptor| descriptor.is_secret).map(|descriptor| descriptor.field_name).collect();
    assert_eq!(listed, names);
    assert_eq!(secret, secrets);
    assert!(descriptors.iter().all(|descriptor| descriptor.db_key == descriptor.field_name && descriptor.env_var == descriptor.field_name));
}

#[test]
fn resolved_fields_keep_descriptor_order() {
    let descriptors = [SLACK_DESCRIPTORS, MS_TEAMS_DESCRIPTORS].concat();
    let environment = BTreeMap::from([(String::from("MS_TEAMS_WEBHOOK_URL"), String::from("https://teams"))]);
    let fields = resolve_fields(&descriptors, &stored(r#"{"SLACK_WEBHOOK_URL": ""}"#), &environment, EmptyDbValue::Cleared).unwrap();
    assert_eq!(
        fields,
        vec![
            ResolvedField { field_name: "SLACK_WEBHOOK_URL", value: Some(String::new()), source: FieldSource::Db },
            ResolvedField { field_name: "MS_TEAMS_WEBHOOK_URL", value: Some(String::from("https://teams")), source: FieldSource::Env },
        ]
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
    store.apply_db_row(DbRow::RouterSettings, stored(r#"{"stored": 2, "missing": null}"#));
    assert_eq!(source_for(&store, key, default.map(json).as_ref()), expected);
}

#[rstest]
#[case::config(FieldSource::Config, r#""config""#)]
#[case::db(FieldSource::Db, r#""db""#)]
#[case::env(FieldSource::Env, r#""env""#)]
#[case::default(FieldSource::Default, r#""default""#)]
#[case::unset(FieldSource::Unset, r#""unset""#)]
fn sources_serialize_as_litellm_names(#[case] source: FieldSource, #[case] expected: &str) {
    assert_eq!(serde_json::to_string(&source).unwrap(), expected);
}
