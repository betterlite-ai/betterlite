use chrono::{DateTime, SecondsFormat, TimeDelta, Utc};
use secrecy::SecretString;

use super::super::types::{CredentialType, OAuthCredentialPayload, OAuthGrantState};
use super::super::{is_oauth_credential_expired, oauth_grant_state};
use super::support::{now, payload};

const BUFFER: i64 = 60;
const PAST: &str =
    r#"{"type":"oauth2","access_token":"tok","expires_at":"2020-01-01T00:00:00+00:00"}"#;
const FUTURE: &str =
    r#"{"type":"oauth2","access_token":"tok","expires_at":"2100-01-01T00:00:00+00:00"}"#;

fn ahead(seconds: i64) -> DateTime<Utc> {
    let offset = TimeDelta::try_seconds(seconds).expect("the offset fits");
    now().checked_add_signed(offset).expect("the moment fits")
}

fn expiring_at(expires: DateTime<Utc>, refresh: bool) -> OAuthCredentialPayload {
    OAuthCredentialPayload {
        access_token: Some(SecretString::from("tok")),
        credential_type: CredentialType::Oauth2,
        refresh_token: refresh.then(|| SecretString::from("ref")),
        expires_at: Some(expires.to_rfc3339_opts(SecondsFormat::Secs, false)),
        connected_at: None,
        scopes: None,
        identity_binding_proof: None,
        server_id: None,
    }
}

fn expired(text: &str, buffer_seconds: i64) -> bool {
    is_oauth_credential_expired(&payload(text), now(), buffer_seconds)
}

fn expires_at(value: &str) -> String {
    format!(r#"{{"type":"oauth2","access_token":"tok","expires_at":"{value}"}}"#)
}

#[test]
fn expiry_treats_a_past_expiry_as_expired() {
    assert!(expired(PAST, 0));
}

#[test]
fn expiry_treats_a_future_expiry_as_live() {
    assert!(!expired(FUTURE, 0));
}

#[test]
fn expiry_buffer_treats_a_token_expiring_inside_the_buffer_as_expired() {
    let credential = expiring_at(ahead(30), false);
    assert!(is_oauth_credential_expired(&credential, now(), 60));
    assert!(!is_oauth_credential_expired(&credential, now(), 0));
}

#[test]
fn expiry_buffer_leaves_a_token_expiring_after_the_buffer_live() {
    let credential = expiring_at(ahead(300), false);
    assert!(!is_oauth_credential_expired(&credential, now(), 60));
}

#[test]
fn expiry_buffer_leaves_a_token_expiring_exactly_at_the_deadline_live() {
    let at_deadline = expiring_at(ahead(60), false);
    assert!(!is_oauth_credential_expired(&at_deadline, now(), 60));
    let just_inside = expiring_at(ahead(59), false);
    assert!(is_oauth_credential_expired(&just_inside, now(), 60));
}

#[test]
fn expiry_buffer_can_be_negative_and_keeps_a_past_token_live() {
    assert!(!expired(PAST, -10_000_000_000));
}

#[test]
fn expiry_absent_expires_at_is_not_expired() {
    assert!(!expired(r#"{"type":"oauth2","access_token":"tok"}"#, 0));
    assert!(!expired(
        r#"{"type":"oauth2","access_token":"tok","expires_at":null}"#,
        0
    ));
}

#[test]
fn expiry_unparsable_expires_at_is_not_expired() {
    assert!(!expired(&expires_at("not-a-date"), 0));
}

#[test]
fn expiry_reads_a_naive_timestamp_as_utc() {
    assert!(expired(&expires_at("2020-01-01T00:00:00"), 0));
    assert!(!expired(&expires_at("2024-01-02T03:04:05"), 0));
    assert!(expired(&expires_at("2024-01-02T03:04:05"), 1));
}

#[test]
fn expiry_reads_the_forms_python_fromisoformat_reads() {
    assert!(expired(&expires_at("2020-01-01T00:00:00Z"), 0));
    assert!(expired(&expires_at("2020-01-01T00:00:00+0000"), 0));
    assert!(expired(&expires_at("2020-01-01 00:00:00+00:00"), 0));
    assert!(expired(&expires_at("2020-01-01 00:00:00Z"), 0));
    assert!(expired(&expires_at("20200101T000000+0000"), 0));
    assert!(expired(&expires_at("2020-01-02T00:00:00,500+00:00"), 0));
}

#[test]
fn expiry_applies_the_offset_of_an_aware_timestamp() {
    assert!(!expired(&expires_at("2024-01-02T04:04:05+01:00"), 0));
    assert!(expired(&expires_at("2024-01-02T04:04:05+01:00"), 1));
    assert!(!expired(&expires_at("2024-01-02T02:04:05-01:00"), 0));
}

#[test]
fn expiry_reads_a_date_alone_as_midnight_utc() {
    assert!(expired(&expires_at("2020-01-01"), 0));
    assert!(!expired(&expires_at("2100-01-01"), 0));
}

#[test]
fn expiry_ignores_a_buffer_beyond_the_calendar_range() {
    assert!(!expired(PAST, i64::MAX));
    assert!(!expired(PAST, 9_000_000_000_000_000));
}

#[test]
fn grant_valid_for_unexpired_token() {
    let credential = payload(FUTURE);
    assert_eq!(
        oauth_grant_state(Some(&credential), now(), BUFFER),
        OAuthGrantState::Valid
    );
}

#[test]
fn grant_refreshable_for_expired_token_with_refresh() {
    let credential = payload(
        r#"{"type":"oauth2","access_token":"tok","refresh_token":"ref","expires_at":"2020-01-01T00:00:00+00:00"}"#,
    );
    assert_eq!(
        oauth_grant_state(Some(&credential), now(), BUFFER),
        OAuthGrantState::Refreshable
    );
}

#[test]
fn grant_absent_without_access_token() {
    let bare = payload(r#"{"type":"oauth2","expires_at":"2100-01-01T00:00:00+00:00"}"#);
    assert_eq!(
        oauth_grant_state(Some(&bare), now(), BUFFER),
        OAuthGrantState::Absent
    );
    assert_eq!(
        oauth_grant_state(None, now(), BUFFER),
        OAuthGrantState::Absent
    );
}

#[test]
fn grant_absent_for_an_expired_token_without_refresh() {
    let credential = payload(PAST);
    assert_eq!(
        oauth_grant_state(Some(&credential), now(), BUFFER),
        OAuthGrantState::Absent
    );
}

#[test]
fn grant_uses_the_buffer_the_caller_passes() {
    let inside = expiring_at(ahead(30), true);
    assert_eq!(
        oauth_grant_state(Some(&inside), now(), BUFFER),
        OAuthGrantState::Refreshable
    );
    assert_eq!(
        oauth_grant_state(Some(&inside), now(), 0),
        OAuthGrantState::Valid
    );
    let inside_without_refresh = expiring_at(ahead(30), false);
    assert_eq!(
        oauth_grant_state(Some(&inside_without_refresh), now(), BUFFER),
        OAuthGrantState::Absent
    );
    let beyond = expiring_at(ahead(300), false);
    assert_eq!(
        oauth_grant_state(Some(&beyond), now(), BUFFER),
        OAuthGrantState::Valid
    );
}

#[test]
fn grant_treats_an_empty_access_token_as_present() {
    let empty =
        payload(r#"{"type":"oauth2","access_token":"","expires_at":"2100-01-01T00:00:00+00:00"}"#);
    assert_eq!(
        oauth_grant_state(Some(&empty), now(), BUFFER),
        OAuthGrantState::Valid
    );
}
