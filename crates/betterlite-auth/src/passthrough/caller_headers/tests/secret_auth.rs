#![cfg(test)]

use rstest::rstest;

use super::caller_filter::{FilterSettings, is_authenticated_caller_secret};
use super::support::{caller, jwt, secret, settings};
use crate::dev_auth_mode::{AuthModeFlags, AuthModeSwitch};
use crate::resolved_key::auth::UserAPIKeyAuth;

fn authenticated(value: &str, auth: &UserAPIKeyAuth, config: &FilterSettings<'_>) -> bool {
    is_authenticated_caller_secret(&secret(value), auth, config)
}

#[rstest]
#[case::bare("mk-1", "mk-1", true)]
#[case::bearer("mk-1", "Bearer mk-1", true)]
#[case::basic("mk-1", "Basic mk-1", true)]
#[case::wrong_key("mk-1", "mk-2", false)]
#[case::empty_value("mk-1", "", false)]
fn the_master_key_matches_in_every_scheme(
    #[case] master: &str,
    #[case] value: &str,
    #[case] expected: bool,
) {
    let master_key = secret(master);
    let auth = caller(None, None);
    let config = settings(
        Some(&master_key),
        AuthModeFlags::default(),
        false,
        None,
        &[],
    );
    assert!(authenticated(value, &auth, &config) == expected);
}

#[rstest]
#[case::bare("stored-key", "stored-key", true)]
#[case::bearer("stored-key", "Bearer stored-key", true)]
#[case::basic("stored-key", "Basic stored-key", true)]
#[case::wrong_key("stored-key", "other-key", false)]
#[case::hashed_sk_key("sk-live-9", "sk-live-9", true)]
#[case::hashed_sk_key_with_bearer("sk-live-9", "Bearer sk-live-9", true)]
fn the_stored_api_key_matches_after_normalization(
    #[case] stored: &str,
    #[case] value: &str,
    #[case] expected: bool,
) {
    let auth = caller(Some(stored), None);
    let config = settings(None, AuthModeFlags::default(), true, None, &[]);
    assert!(authenticated(value, &auth, &config) == expected);
}

#[test]
fn a_caller_without_a_stored_key_matches_nothing() {
    let auth = caller(None, None);
    let config = settings(None, AuthModeFlags::default(), true, None, &[]);
    assert!(!authenticated("stored-key", &auth, &config));
}

#[test]
fn no_auth_dev_mode_without_custom_auth_matches_nothing() {
    let auth = caller(Some("stored-key"), None);
    let config = settings(None, AuthModeFlags::default(), false, None, &[]);
    assert!(!authenticated("stored-key", &auth, &config));
}

#[test]
fn a_configured_custom_auth_compares_the_stored_key_in_dev_mode() {
    let auth = caller(Some("stored-key"), None);
    let config = settings(None, AuthModeFlags::default(), true, None, &[]);
    assert!(authenticated("stored-key", &auth, &config));
}

#[rstest]
#[case::jwt(AuthModeFlags {
    enable_jwt_auth: AuthModeSwitch(true),
    enable_oauth2_auth: AuthModeSwitch(false),
    enable_oauth2_proxy_auth: AuthModeSwitch(false),
})]
#[case::oauth2(AuthModeFlags {
    enable_jwt_auth: AuthModeSwitch(false),
    enable_oauth2_auth: AuthModeSwitch(true),
    enable_oauth2_proxy_auth: AuthModeSwitch(false),
})]
#[case::oauth2_proxy(AuthModeFlags {
    enable_jwt_auth: AuthModeSwitch(false),
    enable_oauth2_auth: AuthModeSwitch(false),
    enable_oauth2_proxy_auth: AuthModeSwitch(true),
})]
fn an_authentication_switch_leaves_dev_mode(#[case] flags: AuthModeFlags) {
    let auth = caller(Some("stored-key"), None);
    let config = settings(None, flags, false, None, &[]);
    assert!(authenticated("stored-key", &auth, &config));
}

#[test]
fn the_jwt_that_authenticated_matches() {
    let stored = r#"{"sub":"user-42"}"#;
    let auth = caller(None, Some(stored));
    let config = settings(None, AuthModeFlags::default(), true, None, &[]);
    assert!(authenticated(&jwt(stored), &auth, &config));
}

#[test]
fn a_token_with_other_claims_does_not_match() {
    let auth = caller(None, Some(r#"{"sub":"user-42"}"#));
    let config = settings(None, AuthModeFlags::default(), true, None, &[]);
    assert!(!authenticated(&jwt(r#"{"sub":"other"}"#), &auth, &config));
}

#[test]
fn an_internal_claim_does_not_change_the_jwt_match() {
    let auth = caller(
        None,
        Some(r#"{"sub":"user-42","_litellm_user_id":"stored"}"#),
    );
    let config = settings(None, AuthModeFlags::default(), true, None, &[]);
    let presented = jwt(r#"{"sub":"user-42","_litellm_user_id":"presented"}"#);
    assert!(authenticated(&presented, &auth, &config));
}

#[test]
fn a_jwt_still_matches_in_no_auth_dev_mode() {
    let stored = r#"{"sub":"user-42"}"#;
    let auth = caller(None, Some(stored));
    let config = settings(None, AuthModeFlags::default(), false, None, &[]);
    assert!(authenticated(&jwt(stored), &auth, &config));
}

#[test]
fn empty_stored_claims_do_not_match_a_token() {
    let auth = caller(Some("stored-key"), Some("{}"));
    let config = settings(None, AuthModeFlags::default(), true, None, &[]);
    assert!(!authenticated(&jwt(r#"{"sub":"x"}"#), &auth, &config));
}

#[test]
fn a_jwt_mismatch_falls_through_to_the_stored_key() {
    let auth = caller(Some("stored-key"), Some(r#"{"sub":"user-42"}"#));
    let config = settings(None, AuthModeFlags::default(), true, None, &[]);
    assert!(authenticated("stored-key", &auth, &config));
}

#[test]
fn the_master_key_matches_before_the_stored_key() {
    let master_key = secret("mk-1");
    let auth = caller(Some("stored-key"), None);
    let config = settings(
        Some(&master_key),
        AuthModeFlags::default(),
        false,
        None,
        &[],
    );
    assert!(authenticated("mk-1", &auth, &config));
}

#[test]
fn an_empty_stored_key_matches_an_empty_value() {
    let auth = caller(Some(""), None);
    let config = settings(None, AuthModeFlags::default(), true, None, &[]);
    assert!(authenticated("", &auth, &config));
}

// docs/deviations/B3603.yaml: LiteLLM's hmac.compare_digest raises on non-ASCII str; betterlite compares UTF-8 bytes
#[test]
fn a_non_ascii_value_compares_as_utf_8_without_raising() {
    let master_key = secret("clé");
    let auth = caller(None, None);
    let config = settings(
        Some(&master_key),
        AuthModeFlags::default(),
        false,
        None,
        &[],
    );
    assert!(authenticated("clé", &auth, &config));
    assert!(!authenticated("cle", &auth, &config));
}
