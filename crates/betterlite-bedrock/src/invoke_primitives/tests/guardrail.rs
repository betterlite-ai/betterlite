use serde_json::value::RawValue;

use crate::invoke_primitives::guardrail::{GuardrailError, guardrail_headers};

fn raw(json_text: &str) -> Box<RawValue> {
    RawValue::from_string(json_text.to_owned()).unwrap()
}

fn headers(json_text: &str) -> Vec<(String, String)> {
    guardrail_headers(&raw(json_text)).unwrap()
}

fn invalid(json_text: &str) -> GuardrailError {
    guardrail_headers(&raw(json_text)).unwrap_err()
}

pub(super) fn guardrail_headers_case() {
    derived_headers();
    empty_values();
    missing_identifier();
    invalid_configurations();
}

fn derived_headers() {
    assert_eq!(
        headers(r#"{"guardrailIdentifier":"id"}"#),
        [(
            "X-Amzn-Bedrock-GuardrailIdentifier".to_owned(),
            "id".to_owned()
        )]
    );
    assert_eq!(
        headers(
            r#"{"guardrailIdentifier":"id","guardrailVersion":"1","trace":"enabled","extra":true}"#
        ),
        [
            (
                "X-Amzn-Bedrock-GuardrailIdentifier".to_owned(),
                "id".to_owned()
            ),
            ("X-Amzn-Bedrock-GuardrailVersion".to_owned(), "1".to_owned()),
            ("X-Amzn-Bedrock-Trace".to_owned(), "ENABLED".to_owned()),
        ]
    );
    assert_eq!(
        headers(r#"{"guardrailIdentifier":"id","trace":"disabled"}"#),
        [
            (
                "X-Amzn-Bedrock-GuardrailIdentifier".to_owned(),
                "id".to_owned()
            ),
            ("X-Amzn-Bedrock-Trace".to_owned(), "DISABLED".to_owned()),
        ]
    );
    assert_eq!(
        headers(r#"{"guardrailIdentifier":"id","trace":"enabled_full"}"#),
        [
            (
                "X-Amzn-Bedrock-GuardrailIdentifier".to_owned(),
                "id".to_owned()
            ),
            ("X-Amzn-Bedrock-Trace".to_owned(), "ENABLED_FULL".to_owned()),
        ]
    );
}

fn empty_values() {
    // an empty identifier and version survive; only absence omits a header
    assert_eq!(
        headers(r#"{"guardrailIdentifier":"","guardrailVersion":""}"#),
        [
            (
                "X-Amzn-Bedrock-GuardrailIdentifier".to_owned(),
                String::new()
            ),
            ("X-Amzn-Bedrock-GuardrailVersion".to_owned(), String::new()),
        ]
    );
}

fn missing_identifier() {
    let missing = invalid(r#"{"guardrailVersion":"1"}"#);
    assert!(matches!(missing, GuardrailError::MissingIdentifier { .. }));
    assert_eq!(missing.status_code(), 400);
    assert!(
        missing
            .to_string()
            .contains("is missing 'guardrailIdentifier'")
    );
    assert!(missing.to_string().contains("Expected format:"));
}

fn invalid_configurations() {
    for value in [
        r#"{"guardrailIdentifier":5}"#,
        r#"{"guardrailIdentifier":"id","guardrailVersion":1}"#,
        r#"{"guardrailIdentifier":null}"#,
        r#"{"guardrailIdentifier":"id","guardrailVersion":null}"#,
        r#"{"guardrailIdentifier":"id","trace":null}"#,
        r#"{"guardrailIdentifier":"id","trace":"bogus"}"#,
        r#"{"guardrailIdentifier":"id","trace":"Absent"}"#,
        r#"{"guardrailIdentifier":"id","trace":5}"#,
        r#""text""#,
        "5",
        "null",
        r#"["a"]"#,
    ] {
        let rejected = invalid(value);
        assert!(
            matches!(rejected, GuardrailError::Invalid { .. }),
            "{value} is not rejected"
        );
        assert_eq!(rejected.status_code(), 400);
        assert!(rejected.to_string().contains("Expected format:"));
    }
}
