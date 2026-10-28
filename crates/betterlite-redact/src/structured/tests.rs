#![cfg(test)]

use std::collections::BTreeSet;

use rstest::rstest;
use serde_json::Value;

use super::{
    SensitiveDataMasker, mask_credentials_in_payload, mask_sensitive_structure,
    redact::{mask_sensitive_keys, redact_credentials_in_payload},
};

fn parse(source: &str) -> Value {
    super::parse::parse_payload(source.as_bytes()).expect("valid JSON fixture")
}

#[rstest]
#[case::sensitive_keys(
    parse(r#"{"api_key":"sk-1234567890abcdef", "model":"gpt-4o", "input_cost_per_token": 0.1}"#),
    parse(r#"{"api_key":"sk-1***************", "model":"gpt-4o", "input_cost_per_token":0.1}"#)
)]
#[case::short_secret(parse(r#"{"token":"short"}"#), parse(r#"{"token":"shor*"}"#))]
#[case::numeric_spelling(
    parse(r#"{"token":1.50,"key":1.0e20}"#),
    parse(r#"{"token":"***","key":"1e+2*"}"#)
)]
#[case::tenancy(
    parse(r#"{"oci_tenancy":"secret"}"#),
    parse(r#"{"oci_tenancy":"secr**"}"#)
)]
#[case::numeric_notation(
    parse(r#"{"token_small":1e-5,"token_tiny":1.2e-7,"token_negative":-1.2345e-5}"#),
    parse(r#"{"token_small":"1e-0*","token_tiny":"1.2e***","token_negative":"-1.2*******"}"#)
)]
#[case::scalar_conversion(
    parse(r#"{"token":null,"auth":true,"password":false,"plain":null,"items":[null,true],"key":[null,false,17,"secret"]}"#),
    parse(r#"{"token":"","auth":"****","password":"\u0046\u0061\u006c\u0073*","plain":"None","items":["None",true],"key":["None",false,17,"secr**"]}"#)
)]
#[case::nested_sequence(
    parse(r#"{"headers":[{"authorization":"Bearer abcdefghijk"}, "plain"]}"#),
    parse(r#"{"headers":[{"authorization":"Bear**************"}, "plain"]}"#)
)]
#[case::excluded_key(
    parse(r#"{"api_key":"secret-value", "token":"token-value"}"#),
    parse(r#"{"api_key":"secr********", "token":"toke*******"}"#)
)]
#[case::nonsecret_shape(
    parse(r#"{"model":"gpt-4o", "max_tokens":17, "nested":{"enabled":true, "empty":null}}"#),
    parse(r#"{"model":"gpt-4o", "max_tokens":17, "nested":{"enabled":true, "empty":"None"}}"#)
)]
fn structured_redaction_behavior_tests_behavior(#[case] input: Value, #[case] expected: Value) {
    let actual = mask_sensitive_structure(&input);
    assert_eq!(actual, expected);
}

#[test]
fn preserves_large_integer_payloads() {
    let source = r#"{"plain":123456789012345678901234567890}"#;
    let payload = parse(source);
    assert_eq!(mask_sensitive_structure(&payload).to_string(), source);
    assert_eq!(mask_credentials_in_payload(&payload).to_string(), source);
}

#[rstest]
#[case::payload_depth(10, mask_credentials_in_payload)]
#[case::redact_depth(101, |data: &Value| redact_credentials_in_payload(data.as_object().unwrap()))]
#[case::hard_depth(100, |data: &Value| SensitiveDataMasker::default().mask_dict(data.as_object().unwrap(), 0, usize::MAX, &[]))]
fn structured_redaction_bounds(#[case] depth: usize, #[case] mask: fn(&Value) -> Value) {
    let mut input = parse(r#"{"api_key":"secret"}"#);
    let mut expected = Value::String("REDACTED".to_owned());
    for level in (1..=depth).rev() {
        input = parse(&format!(r#"{{"l{level}":{input}}}"#));
        expected = parse(&format!(r#"{{"l{level}":{expected}}}"#));
    }
    let output = mask(&input);
    assert_eq!(output, expected);
}

#[test]
fn payload_mask_preserves_shape_and_does_not_mutate_input() {
    let input =
        parse(r#"{"api_key":"sk-secret-123456789", "nested":[{"model":"gpt-4o"}], "none":null}"#);
    let copy = input.clone();
    let output = mask_credentials_in_payload(&input);
    assert_eq!(input, copy);
    assert_eq!(output["nested"][0]["model"], "gpt-4o");
    assert_ne!(output["api_key"], "sk-secret-123456789");
    assert_eq!(output["none"], Value::Null);
}

#[test]
fn exact_key_masking_and_redaction_cover_non_strings() {
    let values = parse(r#"{"token":"short", "other":"short", "none":null}"#);
    let masked = mask_sensitive_keys(
        values.as_object().expect("object"),
        &BTreeSet::from(["token".to_owned()]),
    );
    assert_eq!(masked["token"], "*****");
    assert_eq!(masked["other"], "short");
    let redacted = redact_credentials_in_payload(
        parse(r#"{"token":17, "none":null, "model":"x"}"#)
            .as_object()
            .unwrap(),
    );
    assert_eq!(redacted["token"], "REDACTED");
    assert_eq!(redacted["none"], Value::Null);
}

#[test]
fn custom_patterns_and_overrides_are_local() {
    let masker = SensitiveDataMasker::with_patterns(["connection"], ["cost"]);
    assert!(masker.is_sensitive_key("mongodb_connection_string", &[]));
    assert!(!masker.is_sensitive_key("input_cost_per_token", &[]));
    assert!(!SensitiveDataMasker::default().is_sensitive_key("mongodb_connection_string", &[]));
    assert!(SensitiveDataMasker::default().is_sensitive_key("\u{212a}ey", &[]));
    assert!(
        !SensitiveDataMasker::with_patterns(["connection"], Vec::<String>::new())
            .is_sensitive_key("input_cost_per_token", &[])
    );
}

#[test]
fn excluded_keys_are_honored_by_mask_dict() {
    let masker = SensitiveDataMasker::default();
    let excluded = vec!["api_key".to_owned()];
    let input = parse(r#"{"api_key":"secret-value"}"#);
    let output = masker.mask_dict(input.as_object().expect("object"), 0, 10, &excluded);
    assert_eq!(output["api_key"], "secret-value");
}

#[rstest]
#[case::mapping(r#"{"nested":{"api_key":"secret"}}"#)]
#[case::sequence(r#"{"nested":[{"api_key":"secret"}]}"#)]
fn mask_dict_fails_closed_at_depth(#[case] source: &str) {
    let input = parse(source);
    let masker = SensitiveDataMasker::default();
    let output = masker.mask_dict(input.as_object().unwrap(), 0, 1, &[]);
    assert_eq!(output, parse(r#"{"nested":"REDACTED"}"#));
    assert_eq!(input, parse(source));
}

#[rstest]
#[case::structure(mask_sensitive_structure)]
#[case::payload(mask_credentials_in_payload)]
#[case::redact(|data: &Value| redact_credentials_in_payload(data.as_object().unwrap()))]
fn rejects_excessive_byte_work(#[case] mask: fn(&Value) -> Value) {
    let input = Value::Object(serde_json::Map::from_iter([
        (
            "ordinary".to_owned(),
            Value::String("x".repeat(super::MAX_WORK)),
        ),
        ("token".to_owned(), Value::String("secret".to_owned())),
    ]));
    let output = mask(&input);
    assert_eq!(output, Value::String("REDACTED".to_owned()));
}
