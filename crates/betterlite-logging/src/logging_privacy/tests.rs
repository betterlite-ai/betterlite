#![cfg(test)]

use betterlite_models::decisions::Json;

use super::{
    CallbackMetadataError, PrivacyContext, callback_metadata, redact_user_api_key_info,
    redacted_snapshot, should_redact_message_logging,
};

fn json(source: &str) -> Json {
    serde_json::from_str(source).expect("valid JSON fixture")
}

#[test]
fn logging_privacy_tests_behavior() {
    policy_cases();
    snapshot_cases();
    callback_metadata_cases();
    child_privacy_cases();
}

fn policy_cases() {
    let enable_header = json(r#"{"litellm-enable-message-redaction":true}"#);
    let Json::Object(enable_headers) = &enable_header else {
        panic!("header fixture is an object");
    };
    assert!(!should_redact_message_logging(
        None,
        Some(enable_headers),
        Some(&Json::Bool(false)),
        false
    ));
    assert!(should_redact_message_logging(
        None,
        Some(enable_headers),
        None,
        false
    ));
    let disable_header = json(r#"{"litellm-disable-message-redaction":true}"#);
    let Json::Object(disable_headers) = &disable_header else {
        panic!("header fixture is an object");
    };
    assert!(!should_redact_message_logging(
        None,
        Some(disable_headers),
        Some(&Json::Bool(true)),
        true
    ));
    assert!(should_redact_message_logging(
        None,
        None,
        Some(&Json::Bool(true)),
        false
    ));
    assert!(!should_redact_message_logging(
        None,
        None,
        Some(&Json::Bool(false)),
        true
    ));
    assert!(should_redact_message_logging(None, None, None, true));
    let metadata = json(r#"{"headers":{"litellm-disable-message-redaction":true}}"#);
    assert!(!should_redact_message_logging(
        Some(&metadata),
        None,
        Some(&Json::Bool(true)),
        true
    ));
}

fn snapshot_cases() {
    let payload = json(
        r#"{"messages":[{"content":"secret"}],"prompt":"prompt","input":"input","api_key":"sk-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","response":{"choices":[{"message":{"content":"answer","reasoning_content":"reason","tool_calls":[{"function":{"arguments":"args"}}],"thinking_blocks":["private"],"audio":{"data":"private"}}}]},"nullable":{"content":null}}"#,
    );
    let snapshot = redacted_snapshot(&payload).expect("redaction patterns compile");
    assert_ne!(snapshot, payload);
    let expected = json(
        r#"{"messages":[{"role":"user","content":"[REDACTED]"}],"prompt":"","input":"","api_key":"REDACTED","response":{"choices":[{"message":{"content":"[REDACTED]","reasoning_content":"[REDACTED]","tool_calls":[{"function":{"arguments":"[REDACTED]"}}],"thinking_blocks":null,"audio":null}}]},"nullable":{"content":null}}"#,
    );
    assert_eq!(snapshot, expected);
    assert_eq!(
        payload_field(&payload, "prompt"),
        Some(&Json::Text("prompt".to_owned()))
    );
    let metadata = json(r#"{"user_api_key_hash":"hash","team":"team"}"#);
    assert_eq!(
        redact_user_api_key_info(&metadata, true),
        json(r#"{"team":"team"}"#)
    );
}

fn callback_metadata_cases() {
    let callback = callback_metadata(&json(r#"{"dd_site":"evil","project":"ok"}"#))
        .expect("blocked callback destination is skipped");
    assert_eq!(callback.skipped_fields, ["dd_site"]);
    assert_eq!(
        callback.fields.get("project"),
        Some(&Json::Text("ok".to_owned()))
    );
    assert!(matches!(
        callback_metadata(&json(r#"{"project":"os.environ/SECRET"}"#)),
        Err(CallbackMetadataError::EnvironmentReference { .. })
    ));
    assert!(matches!(
        callback_metadata(&json(r#"{"is_internal_call":true}"#)),
        Err(CallbackMetadataError::TrustedField { .. })
    ));
}

fn child_privacy_cases() {
    assert!(
        PrivacyContext {
            message_logging_disabled: true
        }
        .child(false)
        .message_logging_disabled
    );
    assert!(
        PrivacyContext::default()
            .child(true)
            .message_logging_disabled
    );
}

fn payload_field<'payload>(value: &'payload Json, name: &str) -> Option<&'payload Json> {
    match value {
        Json::Object(fields) => fields.get(name),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}
