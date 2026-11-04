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
}

fn snapshot_cases() {
    let payload =
        json(r#"{"messages":[{"content":"secret"}],"api_key":"sk-abc","prompt":"prompt"}"#);
    let snapshot = redacted_snapshot(&payload).expect("redaction patterns compile");
    assert_ne!(snapshot, payload);
    let original_messages = json(r#"[{"content":"secret"}]"#);
    assert_eq!(
        payload_field(&payload, "messages"),
        Some(&original_messages)
    );
    let content = message_content(&snapshot);
    assert_eq!(content, Some(&Json::Text("[REDACTED]".to_owned())));
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

fn message_content(value: &Json) -> Option<&Json> {
    let Json::Object(fields) = value else {
        return None;
    };
    let Some(Json::List(items)) = fields.get("messages") else {
        return None;
    };
    let Some(Json::Object(message)) = items.first() else {
        return None;
    };
    message.get("content")
}
