#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_core::budget_reset::compute_budget_reset_at;
use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use betterlite_db::identity_rows::PydanticBool;
use betterlite_db::identity_rows::key::Expiry;
use betterlite_db::search_tools::convert::isoformat;
use betterlite_db::team_rows::member::BudgetLimitEntry;
use betterlite_secrets::secret_codec::CALLBACK_VAR_ENCRYPTED_PREFIX;
use rstest::rstest;
use rust_decimal::Decimal;

use super::{Fixture, cipher, fixture, generate_request, instant, object};
use crate::key_fields::values::LiteLLMKeyType;
use crate::key_policy_fields::generate::{effective_key_for_generate, generate_budget_windows};
use crate::key_policy_fields::key_type::handle_key_type;
use crate::key_policy_fields::microsecond_precision;

fn reset_text(fixture: &Fixture, spec: &str) -> String {
    let reset =
        compute_budget_reset_at(spec, &fixture.reset, fixture.now, &fixture.database).unwrap();
    isoformat(microsecond_precision(reset.instant))
}

// the logging slot's callback vars are encrypted, so the plain api key never reaches the row
fn assert_callback_vars_encrypted(metadata: &BTreeMap<String, JsonData>) {
    assert_eq!(
        metadata.get("end_user_budget_id"),
        Some(&super::json(r#""eu-1""#))
    );
    // an explicitly null request field is excluded before the metadata fold
    assert!(!metadata.contains_key("prompts"));
    let Some(JsonData::Array(entries)) = metadata.get("logging") else {
        panic!("the logging slot stays a list");
    };
    let Some(JsonData::Object(entry)) = entries.first() else {
        panic!("the callback entry stays an object");
    };
    let Some(JsonData::Object(vars)) = entry.get("callback_vars") else {
        panic!("the callback vars stay an object");
    };
    let Some(JsonData::String(api_key)) = vars.get("api_key") else {
        panic!("the api key stays text");
    };
    assert!(
        api_key.starts_with(CALLBACK_VAR_ENCRYPTED_PREFIX),
        "{api_key} is not encrypted"
    );
    assert_ne!(api_key, "sk-plain");
    // a key no sensitive list names stays readable
    assert_eq!(
        vars.get("endpoint"),
        Some(&super::json(r#""https://example.test""#))
    );
}

#[test]
fn generated_metadata_override_encrypted() {
    let fixture = fixture();
    let request = generate_request(
        r#"{"key_alias":"alias","prompts":null,"tags":["new"],"end_user_budget_id":"eu-1",
            "metadata":{"keep":"value","tags":["old"],
              "logging":[{"callback_name":"custom","callback_vars":{"api_key":"sk-plain","endpoint":"https://example.test"}}]}}"#,
    );
    let token = effective_key_for_generate(&request, fixture.clock(), &cipher()).unwrap();
    assert_eq!(token.key_alias.as_deref(), Some("alias"));
    // the request field wins over the same key inside metadata
    assert_eq!(
        token.metadata.0.get("tags"),
        Some(&super::json(r#"["new"]"#))
    );
    assert_eq!(
        token.metadata.0.get("keep"),
        Some(&super::json(r#""value""#))
    );
    assert_callback_vars_encrypted(&token.metadata.0);
    // the request named no key type, so the model default applies
    assert_eq!(token.key_type.as_deref(), Some("default"));
}

#[test]
fn generate_clock_expiry_and_rotation() {
    let fixture = fixture();
    let request = generate_request(
        r#"{"duration":"2h","budget_duration":"1d","auto_rotate":true,"rotation_interval":"30m"}"#,
    );
    let token = effective_key_for_generate(&request, fixture.clock(), &cipher()).unwrap();
    assert_eq!(
        token.expires,
        Some(Expiry::Text("2026-05-01T12:00:00.123456Z".to_owned()))
    );
    assert_eq!(
        token.budget_reset_at.map(Timestamp::utc),
        Some(
            compute_budget_reset_at("1d", &fixture.reset, fixture.now, &fixture.database)
                .unwrap()
                .instant
                .with_timezone(&chrono::Utc)
        )
    );
    assert_eq!(
        token.key_rotation_at.map(Timestamp::utc),
        Some(instant("2026-05-01T10:30:00.123456Z"))
    );
    assert_eq!(token.rotation_interval.as_deref(), Some("30m"));
    assert_eq!(token.auto_rotate.0, Some(PydanticBool(true)));
}

#[rstest]
#[case::rotation_off(
    r#"{"auto_rotate":false,"rotation_interval":"30m"}"#,
    Some("30m"),
    false
)]
#[case::interval_missing(r#"{"auto_rotate":true}"#, None, true)]
#[case::interval_empty(r#"{"auto_rotate":true,"rotation_interval":""}"#, Some(""), true)]
#[case::no_timing_fields(r#"{"key_alias":"alias"}"#, None, false)]
fn generate_without_rotation_keeps_timing_fields_empty(
    #[case] document: &str,
    #[case] interval: Option<&str>,
    #[case] auto_rotate: bool,
) {
    let fixture = fixture();
    let token = effective_key_for_generate(&generate_request(document), fixture.clock(), &cipher())
        .unwrap();
    assert!(token.expires.is_none());
    assert!(token.budget_reset_at.is_none());
    assert!(token.key_rotation_at.is_none());
    // the supplied rotation fields still pass through, only the schedule stays unset
    assert_eq!(token.rotation_interval.as_deref(), interval);
    assert_eq!(token.auto_rotate.0, Some(PydanticBool(auto_rotate)));
}

#[test]
fn budget_windows_empty_and_order() {
    let fixture = fixture();
    assert_eq!(
        generate_budget_windows(None, fixture.clock()).unwrap(),
        None
    );
    assert_eq!(
        generate_budget_windows(Some(&[]), fixture.clock()).unwrap(),
        None
    );
    let windows = [
        BudgetLimitEntry {
            budget_duration: "1d".to_owned(),
            max_budget: Decimal::from(10_i32),
            reset_at: None,
        },
        BudgetLimitEntry {
            budget_duration: "1h".to_owned(),
            max_budget: Decimal::new(25, 1),
            reset_at: None,
        },
    ];
    let built = generate_budget_windows(Some(&windows), fixture.clock())
        .unwrap()
        .expect("two windows stay two windows");
    assert_windows(&fixture, &built);
}

fn assert_windows(fixture: &Fixture, built: &[JsonData]) {
    assert_eq!(built.len(), 2);
    let Some(JsonData::Object(first)) = built.first() else {
        panic!("a window is an object");
    };
    let Some(JsonData::Object(second)) = built.get(1) else {
        panic!("a window is an object");
    };
    assert_eq!(first.get("budget_duration"), Some(&super::json(r#""1d""#)));
    assert_eq!(
        first.get("reset_at"),
        Some(&JsonData::String(reset_text(fixture, "1d")))
    );
    assert_eq!(second.get("budget_duration"), Some(&super::json(r#""1h""#)));
    assert_eq!(
        second.get("reset_at"),
        Some(&JsonData::String(reset_text(fixture, "1h")))
    );
    // the window's own fields survive the reset time
    assert!(matches!(first.get("max_budget"), Some(JsonData::Number(_))));
}

#[test]
fn key_type_absent_removes_the_column() {
    let columns = handle_key_type(None, object(r#"{"key_type":"llm_api","other":1}"#));
    assert!(!columns.contains_key("key_type"));
    assert_eq!(columns.get("other"), Some(&super::json("1")));
}

#[rstest]
#[case::llm_api(LiteLLMKeyType::LlmApi, "llm_api", "llm_api_routes")]
#[case::management(LiteLLMKeyType::Management, "management", "management_routes")]
#[case::read_only(LiteLLMKeyType::ReadOnly, "read_only", "info_routes")]
fn key_type_route_presets(
    #[case] key_type: LiteLLMKeyType,
    #[case] text: &str,
    #[case] route: &str,
) {
    let columns = handle_key_type(Some(key_type), object(r#"{"allowed_routes":["custom"]}"#));
    assert_eq!(
        columns.get("key_type"),
        Some(&JsonData::String(text.to_owned()))
    );
    assert_eq!(
        columns.get("allowed_routes"),
        Some(&super::json(&format!(r#"["{route}"]"#)))
    );
}

#[test]
fn explicit_null_key_type_leaves_the_column_out() {
    let fixture = fixture();
    let request = generate_request(r#"{"key_type":null,"key_alias":"alias"}"#);
    let token = effective_key_for_generate(&request, fixture.clock(), &cipher()).unwrap();
    assert!(token.key_type.is_none());
}

#[test]
fn default_key_type_keeps_routes() {
    let columns = handle_key_type(
        Some(LiteLLMKeyType::Default),
        object(r#"{"allowed_routes":["custom"]}"#),
    );
    assert_eq!(
        columns.get("key_type"),
        Some(&JsonData::String("default".to_owned()))
    );
    assert_eq!(
        columns.get("allowed_routes"),
        Some(&super::json(r#"["custom"]"#))
    );
}
