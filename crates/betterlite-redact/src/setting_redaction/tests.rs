#![cfg(test)]

use rstest::rstest;

use super::{
    dump_redacted_config, is_secret_setting, redact_param_for_logging, redact_secret_values,
    redact_setting_value,
};
use crate::structured::Payload;

fn parsed(source: &str) -> Payload {
    serde_json::from_str(source).expect("valid JSON fixture")
}

fn redacted() -> Payload {
    parsed(r#""REDACTED""#)
}

// the innermost value of a chain of `depth` objects sits at that same walk depth
fn nested(depth: usize, leaf: &str) -> Payload {
    let mut value = parsed(leaf);
    for level in (1..=depth).rev() {
        value = parsed(&format!(r#"{{"l{level}":{value}}}"#));
    }
    value
}

#[rstest]
#[case::database_url("database_url")]
#[case::database_extra_connection_params("database_extra_connection_params")]
#[case::pass_through_endpoints("pass_through_endpoints")]
#[case::alert_to_webhook_url("alert_to_webhook_url")]
#[case::master_key("master_key")]
#[case::authorization_header("authorization")]
#[case::credentials("credentials")]
fn is_secret_setting_flags_the_extra_and_the_masker_names(#[case] name: &str) {
    assert!(is_secret_setting(name));
}

#[rstest]
#[case::model("model")]
#[case::database_args("database_args")]
#[case::database_host("database_host")]
#[case::environment_variables("environment_variables")]
#[case::input_cost("input_cost_per_token")]
fn is_secret_setting_allows_plain_names(#[case] name: &str) {
    assert!(!is_secret_setting(name));
}

#[rstest]
#[case::nested_identity_token(
    parsed(r#"{"database_args":{"aws_web_identity_token":"tok-123","host":"db.example.com"},"model":"gpt-4o"}"#),
    parsed(r#"{"database_args":{"aws_web_identity_token":"REDACTED","host":"db.example.com"},"model":"gpt-4o"}"#)
)]
#[case::extra_flagged_field(
    parsed(r#"{"database_url":"postgres://user:pass@host/db"}"#),
    parsed(r#"{"database_url":"REDACTED"}"#)
)]
#[case::masker_flagged_field(
    parsed(r#"{"master_key":"sk-live"}"#),
    parsed(r#"{"master_key":"REDACTED"}"#)
)]
#[case::null_secret(
    parsed(r#"{"password":null,"model":"gpt-4o"}"#),
    parsed(r#"{"password":"REDACTED","model":"gpt-4o"}"#)
)]
#[case::array_of_objects(
    parsed(r#"[{"api_key":"sk-live"},{"model":"gpt-4o"}]"#),
    parsed(r#"[{"api_key":"REDACTED"},{"model":"gpt-4o"}]"#)
)]
#[case::scalars_unchanged(
    parsed(r#"{"model":"gpt-4o","max_tokens":17,"stream":true,"temperature":null}"#),
    parsed(r#"{"model":"gpt-4o","max_tokens":17,"stream":true,"temperature":null}"#)
)]
fn redact_secret_values_hides_secret_leaves(#[case] value: Payload, #[case] expected: Payload) {
    assert_eq!(redact_secret_values(&value), expected);
}

#[test]
fn redact_secret_values_replaces_a_scalar_at_ten_levels() {
    assert_eq!(
        redact_secret_values(&nested(10, r#""leaf""#)),
        nested(10, r#""REDACTED""#)
    );
}

#[test]
fn redact_secret_values_drops_a_subtree_below_ten_levels() {
    assert_eq!(
        redact_secret_values(&nested(11, r#""leaf""#)),
        nested(10, r#""REDACTED""#)
    );
}

#[test]
fn redact_secret_values_leaves_the_callers_value_alone() {
    let value = parsed(r#"{"master_key":"sk-live","model":"gpt-4o"}"#);
    let untouched = value.clone();
    let redacted_output = redact_secret_values(&value);
    assert_eq!(value, untouched);
    assert_eq!(
        redacted_output,
        parsed(r#"{"master_key":"REDACTED","model":"gpt-4o"}"#)
    );
}

#[test]
fn redact_param_for_logging_hides_every_environment_variable_value() {
    let value = parsed(r#"{"DATABASE_URL":"postgres://user:pass@host/db","PLAIN":"one"}"#);
    assert_eq!(
        redact_param_for_logging(Some("environment_variables"), &value),
        parsed(r#"{"DATABASE_URL":"REDACTED","PLAIN":"REDACTED"}"#)
    );
}

#[test]
fn redact_param_for_logging_walks_any_other_name_by_key() {
    let value = parsed(r#"{"DATABASE_URL":"postgres://user:pass@host/db","api_key":"sk-live"}"#);
    assert_eq!(
        redact_param_for_logging(Some("general_settings"), &value),
        parsed(r#"{"DATABASE_URL":"postgres://user:pass@host/db","api_key":"REDACTED"}"#)
    );
}

#[test]
fn redact_param_for_logging_keeps_a_non_object_environment_variables_value() {
    let value = parsed(r#"[{"DATABASE_URL":"postgres://user:pass@host/db"}]"#);
    assert_eq!(
        redact_param_for_logging(Some("environment_variables"), &value),
        value
    );
}

#[test]
fn redact_param_for_logging_passes_a_scalar_and_an_unnamed_value_through() {
    let value = parsed(r#""plain""#);
    assert_eq!(redact_param_for_logging(None, &value), value);
    assert_eq!(redact_param_for_logging(Some("model"), &value), value);
}

#[test]
fn redact_setting_value_passes_everything_through_for_a_full_admin() {
    let value = parsed(r#"{"database_url":"postgres://user:pass@host/db"}"#);
    assert_eq!(redact_setting_value("database_url", &value, true), value);
    assert_eq!(
        redact_setting_value("environment_variables", &value, true),
        value
    );
}

#[test]
fn redact_setting_value_redacts_a_secret_field_whole() {
    let value = parsed(r#"[{"headers":{"Authorization":"Bearer abc"}}]"#);
    assert_eq!(
        redact_setting_value("pass_through_endpoints", &value, false),
        redacted()
    );
    assert_eq!(
        redact_setting_value("database_url", &parsed(r#""postgres://host/db""#), false),
        redacted()
    );
}

#[test]
fn redact_setting_value_walks_an_ordinary_field() {
    let value =
        parsed(r#"{"database_args":{"aws_web_identity_token":"tok-123"},"model":"gpt-4o"}"#);
    assert_eq!(
        redact_setting_value("general_settings", &value, false),
        parsed(r#"{"database_args":{"aws_web_identity_token":"REDACTED"},"model":"gpt-4o"}"#)
    );
}

#[test]
fn dump_redacted_config_keeps_an_absent_value_absent() {
    assert_eq!(dump_redacted_config(None, false), None);
    assert_eq!(dump_redacted_config(None, true), None);
}

#[test]
fn dump_redacted_config_hides_every_value_when_asked() {
    let value = parsed(r#"{"DATABASE_URL":"postgres://user:pass@host/db","PLAIN":"one"}"#);
    let dumped = dump_redacted_config(Some(&value), true).expect("a JSON value serializes");
    assert_eq!(
        parsed(&dumped),
        parsed(r#"{"DATABASE_URL":"REDACTED","PLAIN":"REDACTED"}"#)
    );
}

#[test]
fn dump_redacted_config_walks_secret_keys_otherwise() {
    let value = parsed(r#"{"database_url":"postgres://user:pass@host/db","model":"gpt-4o"}"#);
    let dumped = dump_redacted_config(Some(&value), false).expect("a JSON value serializes");
    assert_eq!(
        parsed(&dumped),
        parsed(r#"{"database_url":"REDACTED","model":"gpt-4o"}"#)
    );
}

#[test]
fn dump_redacted_config_writes_a_null_value_as_null_text() {
    let value = parsed("null");
    let dumped = dump_redacted_config(Some(&value), false).expect("a JSON value serializes");
    assert_eq!(parsed(&dumped), parsed("null"));
}
