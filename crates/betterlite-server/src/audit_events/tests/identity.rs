#![cfg(test)]

use betterlite_config::sections::settings::JsonValue;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::json;

use super::support::parsed;
use crate::audit_events::identity::{CallerIdentity, get_audit_log_changed_by};

fn caller() -> CallerIdentity {
    CallerIdentity {
        user_id: Some("user-1".to_owned()),
        api_key: Some(SecretString::from("sk-key".to_owned())),
        metadata: None,
        team_metadata: None,
    }
}

#[test]
fn audit_identity_debug_redacts_the_api_key() {
    let text = format!("{:?}", caller());
    assert!(text.contains("[REDACTED]"));
    assert!(!text.contains("sk-key"));
}

#[test]
fn audit_header_opt_in() {
    let mut metadata = caller();
    metadata.metadata = Some(parsed(r#"{"allow_litellm_changed_by_header": true}"#));
    assert_eq!(
        get_audit_log_changed_by(Some("ops"), &metadata, Some("admin")),
        Some("ops".to_owned())
    );
    let mut team = caller();
    team.team_metadata = Some(parsed(r#"{"allow_litellm_changed_by_header": true}"#));
    assert_eq!(
        get_audit_log_changed_by(Some("ops"), &team, None),
        Some("ops".to_owned())
    );
    let mut both = caller();
    both.metadata = Some(parsed(r#"{"allow_litellm_changed_by_header": false}"#));
    both.team_metadata = Some(parsed(r#"{"allow_litellm_changed_by_header": true}"#));
    assert_eq!(
        get_audit_log_changed_by(Some("ops"), &both, None),
        Some("ops".to_owned())
    );
}

#[test]
fn audit_header_falsey() {
    let mut empty = caller();
    empty.metadata = Some(parsed(r#"{"allow_litellm_changed_by_header": true}"#));
    assert_eq!(
        get_audit_log_changed_by(Some(""), &empty, Some("admin")),
        Some("user-1".to_owned())
    );
    let mut denied = caller();
    denied.metadata = Some(parsed(r#"{"allow_litellm_changed_by_header": false}"#));
    assert_eq!(
        get_audit_log_changed_by(Some("ops"), &denied, Some("admin")),
        Some("user-1".to_owned())
    );
    let mut missing = caller();
    missing.metadata = Some(parsed(r#"{"other": true}"#));
    assert_eq!(
        get_audit_log_changed_by(Some("ops"), &missing, Some("admin")),
        Some("user-1".to_owned())
    );
    let mut elsewhere = caller();
    elsewhere.metadata = Some(parsed(
        r#"{"nested": {"allow_litellm_changed_by_header": true}}"#,
    ));
    assert_eq!(
        get_audit_log_changed_by(Some("ops"), &elsewhere, Some("admin")),
        Some("user-1".to_owned())
    );
}

#[rstest]
#[case::string(json!("true"))]
#[case::integer(json!(1))]
#[case::float(json!(1.0))]
#[case::array(json!([true]))]
fn audit_header_truthy_nonbool_denied(#[case] value: JsonValue) {
    let mut metadata = caller();
    let text = format!(r#"{{"allow_litellm_changed_by_header": {value}}}"#);
    metadata.metadata = Some(parsed(&text));
    assert_eq!(
        get_audit_log_changed_by(Some("ops"), &metadata, Some("admin")),
        Some("user-1".to_owned())
    );
}

#[test]
fn audit_identity_fallback() {
    assert_eq!(
        get_audit_log_changed_by(None, &caller(), Some("admin")),
        Some("user-1".to_owned())
    );
    let mut admin_only = caller();
    admin_only.user_id = None;
    assert_eq!(
        get_audit_log_changed_by(None, &admin_only, Some("admin")),
        Some("admin".to_owned())
    );
    let mut blank = caller();
    blank.user_id = Some(String::new());
    assert_eq!(
        get_audit_log_changed_by(None, &blank, Some("admin")),
        Some("admin".to_owned())
    );
    assert_eq!(get_audit_log_changed_by(None, &admin_only, None), None);
    assert_eq!(
        get_audit_log_changed_by(Some("ops"), &caller(), None),
        Some("user-1".to_owned())
    );
}
