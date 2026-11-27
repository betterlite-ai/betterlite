#![cfg(test)]

use std::collections::BTreeSet;

use betterlite_config::sections::settings::JsonValue;
use rstest::rstest;

use super::super::SettingsMap;
use super::super::merge;
use super::support::object;

fn callback_set(merged: &SettingsMap) -> BTreeSet<String> {
    merged
        .get("success_callback")
        .and_then(JsonValue::as_array)
        .expect("the stored callbacks")
        .iter()
        .filter_map(|callback| callback.as_str().map(str::to_owned))
        .collect()
}

#[rstest]
#[case::absent_alerting(
    r#"{}"#,
    r#"{"alert_to_webhook_url": {"budget": "https://x"}}"#,
    r#"{"alerting": ["slack"], "alert_to_webhook_url": {"budget": "https://x"}}"#
)]
#[case::appends_to_the_stored_list(
    r#"{"alerting": ["email"]}"#,
    r#"{"alert_to_webhook_url": {"budget": "https://x"}}"#,
    r#"{"alerting": ["email", "slack"], "alert_to_webhook_url": {"budget": "https://x"}}"#
)]
#[case::keeps_an_existing_slack_entry(
    r#"{"alerting": ["slack"]}"#,
    r#"{"alert_to_webhook_url": {"budget": "https://x"}}"#,
    r#"{"alerting": ["slack"], "alert_to_webhook_url": {"budget": "https://x"}}"#
)]
#[case::leaves_a_non_list_untouched(
    r#"{"alerting": "email"}"#,
    r#"{"alert_to_webhook_url": {"budget": "https://x"}}"#,
    r#"{"alerting": "email", "alert_to_webhook_url": {"budget": "https://x"}}"#
)]
#[case::the_sent_alerting_list_gets_slack(
    r#"{}"#,
    r#"{"alerting": ["email"], "alert_to_webhook_url": {"budget": "https://x"}}"#,
    r#"{"alerting": ["email", "slack"], "alert_to_webhook_url": {"budget": "https://x"}}"#
)]
#[case::other_settings_request_wins(
    r#"{"store_model_in_db": true, "alerting": ["email"]}"#,
    r#"{"store_model_in_db": false}"#,
    r#"{"store_model_in_db": false, "alerting": ["email"]}"#
)]
fn general_settings_merge(#[case] existing: &str, #[case] sent: &str, #[case] expected: &str) {
    assert_eq!(
        merge::general_settings(&object(existing), &object(sent)),
        object(expected)
    );
}

#[test]
fn without_nulls_drops_only_top_level_nulls() {
    assert_eq!(
        merge::without_nulls(&object(r#"{"a": null, "b": {"c": null}, "d": 1}"#)),
        object(r#"{"b": {"c": null}, "d": 1}"#)
    );
}

#[test]
fn litellm_success_callback_is_normalized_and_unioned_with_the_stored_list() {
    let merged = merge::litellm_settings(
        &object(r#"{"success_callback": ["S3", "custom"]}"#),
        &object(r#"{"success_callback": ["New", "s3"], "drop_params": true}"#),
    );
    assert_eq!(
        callback_set(&merged),
        BTreeSet::from(["custom".to_owned(), "new".to_owned(), "s3".to_owned(),])
    );
    assert_eq!(merged.get("drop_params"), Some(&JsonValue::Bool(true)));
}

#[test]
fn litellm_callback_without_a_stored_list_is_the_incoming_set() {
    let merged = merge::litellm_settings(
        &object(r#"{"drop_params": false}"#),
        &object(r#"{"success_callback": ["A", "a"]}"#),
    );
    assert_eq!(callback_set(&merged), BTreeSet::from(["a".to_owned()]));
    assert_eq!(merged.get("drop_params"), Some(&JsonValue::Bool(false)));
}

#[test]
fn litellm_keeps_a_success_callback_that_is_not_a_list() {
    let merged =
        merge::litellm_settings(&object(r#"{}"#), &object(r#"{"success_callback": "S3"}"#));
    assert_eq!(
        merged.get("success_callback"),
        Some(&JsonValue::String("S3".to_owned()))
    );
}

#[test]
fn litellm_callbacks_keep_one_entry_per_name_with_the_stored_names_first() {
    let merged = merge::litellm_settings(
        &object(r#"{"success_callback": ["S3"]}"#),
        &object(r#"{"success_callback": ["s3", "New"]}"#),
    );
    assert_eq!(
        merged.get("success_callback"),
        Some(&JsonValue::Array(vec![
            JsonValue::String("s3".to_owned()),
            JsonValue::String("new".to_owned()),
        ]))
    );
}

#[test]
fn environment_variables_overwrite_only_the_sent_keys() {
    assert_eq!(
        merge::environment_variables(&object(r#"{"A": "x", "B": "y"}"#), &object(r#"{"B": "z"}"#),),
        object(r#"{"A": "x", "B": "z"}"#)
    );
}

#[test]
fn router_updates_overwrite_the_stored_keys() {
    assert_eq!(
        merge::router_settings(
            &object(r#"{"num_retries": 1, "timeout": 30.0}"#),
            &object(r#"{"timeout": 5.0}"#),
        ),
        object(r#"{"num_retries": 1, "timeout": 5.0}"#)
    );
}
