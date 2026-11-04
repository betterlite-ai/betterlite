#![cfg(test)]

use serde_json::json;

use super::{
    CallbackMetadataError, PrivacyContext, callback_metadata, redact_user_api_key_info,
    redacted_snapshot, should_redact_message_logging,
};

#[test]
fn logging_privacy_tests_behavior() {
    let headers = json!({"litellm-enable-message-redaction": true});
    assert!(!should_redact_message_logging(
        None,
        headers.as_object(),
        Some(&json!(false)),
        false
    ));
    let disabled = json!({"litellm-disable-message-redaction": true});
    assert!(!should_redact_message_logging(
        None,
        disabled.as_object(),
        Some(&json!(true)),
        true
    ));
    assert!(should_redact_message_logging(
        None,
        None,
        Some(&json!(true)),
        false
    ));
    assert!(should_redact_message_logging(
        None,
        None,
        Some(&json!(false)),
        true
    ));
    let payload = json!({"messages":[{"content":"secret"}],"api_key":"sk-abc","prompt":"prompt"});
    let snapshot = redacted_snapshot(&payload);
    assert_ne!(snapshot, payload);
    assert_eq!(payload["messages"][0]["content"], "secret");
    assert_eq!(snapshot["messages"][0]["content"], "[REDACTED]");
    let metadata = json!({"user_api_key_hash":"hash","team":"team"});
    assert_eq!(
        redact_user_api_key_info(&metadata, true),
        json!({"team":"team"})
    );
    let callback = callback_metadata(&json!({"dd_site":"evil","project":"ok"}))
        .expect("blocked callback destination is skipped");
    assert_eq!(callback.skipped_fields, ["dd_site"]);
    assert_eq!(
        callback.fields,
        json!({"project":"ok"})
            .as_object()
            .cloned()
            .expect("JSON object")
    );
    assert!(matches!(
        callback_metadata(&json!({"project":"os.environ/SECRET"})),
        Err(CallbackMetadataError::EnvironmentReference { .. })
    ));
    assert!(matches!(
        callback_metadata(&json!({"is_internal_call":true})),
        Err(CallbackMetadataError::TrustedField { .. })
    ));
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
