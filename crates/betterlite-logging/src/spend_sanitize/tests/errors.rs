use betterlite_models::decisions::Json;

use crate::spend_sanitize::REDACTED_BY_LITELM;
use crate::spend_sanitize::SpendSanitizePolicy;
use crate::spend_sanitize::errors::sanitize_error_information;
use crate::spend_sanitize::value::SpendValue;

use super::support::{lookup, number, object, value};

fn policy(store_bodies: bool, max_string_length: i64) -> SpendSanitizePolicy {
    SpendSanitizePolicy {
        store_bodies,
        max_string_length,
    }
}

fn sanitize(information: SpendValue, store_bodies: bool, max: i64) -> Json {
    let SpendValue::Mapping(mapping) = information else {
        panic!("error information is a mapping")
    };
    sanitize_error_information(&mapping, &policy(store_bodies, max), None)
}

fn redacted() -> Json {
    Json::Text(REDACTED_BY_LITELM.to_owned())
}

#[test]
fn redacts_prompt_leaks_only_when_bodies_are_disabled() {
    let source = value(
        r#"{"error_message":"{\"input\": [{\"content\": \"hi\"}]}","traceback":"input_value='secret'","error_code":400}"#,
    );
    let disabled = sanitize(source.clone(), false, 2048);
    assert_eq!(
        lookup(&disabled, "error_message"),
        &Json::Text(r#"{"input": "REDACTED_BY_LITELM"}"#.to_owned())
    );
    assert_eq!(
        lookup(&disabled, "traceback"),
        &Json::Text("input_value=\"REDACTED_BY_LITELM\"".to_owned())
    );
    let stored = sanitize(source, true, 2048);
    assert_eq!(
        lookup(&stored, "error_message"),
        &Json::Text(r#"{"input": [{"content": "hi"}]}"#.to_owned())
    );
    assert_eq!(
        lookup(&stored, "traceback"),
        &Json::Text("input_value='secret'".to_owned())
    );
}

#[test]
fn uses_the_safe_spend_message_when_supplied() {
    let source =
        value(r#"{"error_message":"raw model detail","error_class":"ProxyModelNotFoundError"}"#);
    let SpendValue::Mapping(mapping) = source else {
        panic!("error information is a mapping")
    };
    let policy = policy(false, 2048);
    let sanitized = sanitize_error_information(
        &mapping,
        &policy,
        Some("acompletion: Invalid model name passed in."),
    );
    assert_eq!(
        lookup(&sanitized, "error_message"),
        &Json::Text("acompletion: Invalid model name passed in.".to_owned())
    );
    assert_eq!(
        lookup(&sanitized, "error_class"),
        &Json::Text("ProxyModelNotFoundError".to_owned())
    );
}

#[test]
fn inserts_the_safe_spend_message_when_the_field_is_absent() {
    let source = value(r#"{"error_class":"ProxyModelNotFoundError"}"#);
    let SpendValue::Mapping(mapping) = source else {
        panic!("error information is a mapping")
    };
    let policy = policy(false, 2048);
    let sanitized = sanitize_error_information(&mapping, &policy, Some("safe message"));
    assert_eq!(
        lookup(&sanitized, "error_message"),
        &Json::Text("safe message".to_owned())
    );
}

#[test]
fn applies_the_storage_sanitizer_to_the_remaining_fields() {
    let source = value(
        r#"{"error_message":"abcdefghij","secret_fields":{"authorization":"Bearer x"},"api_key":"sk-live-1","error_code":400}"#,
    );
    let sanitized = sanitize(source, false, 4);
    let Json::Text(truncated) = lookup(&sanitized, "error_message") else {
        panic!("error_message stays text")
    };
    assert!(truncated.starts_with('a'));
    assert!(truncated.ends_with("ij"));
    assert!(truncated.contains("litellm_truncated"));
    assert!(!object(&sanitized).contains_key("secret_fields"));
    assert_eq!(lookup(&sanitized, "api_key"), &redacted());
    assert_eq!(lookup(&sanitized, "error_code"), &number(400));
}

#[test]
fn keeps_null_and_non_string_fields() {
    let source = value(r#"{"error_message":null,"traceback":17,"error_class":"X"}"#);
    let sanitized = sanitize(source, false, 2048);
    assert_eq!(lookup(&sanitized, "error_message"), &Json::Null);
    assert_eq!(lookup(&sanitized, "traceback"), &number(17));
    assert_eq!(
        lookup(&sanitized, "error_class"),
        &Json::Text("X".to_owned())
    );
}

#[test]
fn keeps_a_mapping_without_prompt_fields_unchanged() {
    let source = value(r#"{"error_class":"X"}"#);
    let sanitized = sanitize(source, false, 2048);
    let Json::Object(entries) = &sanitized else {
        panic!("error information stays an object")
    };
    assert_eq!(entries.len(), 1);
    assert_eq!(entries["error_class"], Json::Text("X".to_owned()));
}
