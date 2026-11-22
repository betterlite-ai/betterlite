#![cfg(test)]

use secrecy::{ExposeSecret as _, SecretString};

use super::{AuthCodeRecord, SinkError};

fn record() -> AuthCodeRecord {
    AuthCodeRecord {
        api_key: SecretString::from("sk-user-secret".to_owned()),
        server_id: "srv-1".to_owned(),
        code_challenge: "challenge".to_owned(),
        redirect_uri: "http://127.0.0.1:1456/callback".to_owned(),
        client_id: "client-1".to_owned(),
        user_id: "user-1".to_owned(),
        expires_at: 1_761_000_300.0,
    }
}

#[test]
fn auth_code_record_debug_redacts_the_api_key() {
    let debugged = format!("{:?}", record());
    assert!(!debugged.contains("sk-user-secret"));
    assert!(debugged.contains("user-1"));
}

#[test]
fn auth_code_record_clone_keeps_the_api_key() {
    let original = record();
    let cloned = original.clone();
    assert_eq!(cloned.api_key.expose_secret(), "sk-user-secret");
}

#[test]
fn sink_error_display_names_the_failing_operation() {
    let store = SinkError::Store {
        message: "connection refused".to_owned(),
    };
    assert_eq!(
        store.to_string(),
        "the credential could not be stored: connection refused"
    );
    let invalidate = SinkError::Invalidate {
        message: "peer publish failed".to_owned(),
    };
    assert_eq!(
        invalidate.to_string(),
        "the credential cache could not be invalidated: peer publish failed"
    );
}
