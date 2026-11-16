use chrono::{DateTime, SecondsFormat, TimeDelta, Utc};

use super::super::types::{CredentialType, OAuthCredentialPayload, OAuthGrantState};
use super::super::{
    MCP_PER_USER_TOKEN_EXPIRY_BUFFER_SECONDS, is_oauth_credential_expired, oauth_grant_state,
};
use super::support::payload;

const PAST: &str =
    r#"{"type":"oauth2","access_token":"tok","expires_at":"2020-01-01T00:00:00+00:00"}"#;
const FUTURE: &str =
    r#"{"type":"oauth2","access_token":"tok","expires_at":"2100-01-01T00:00:00+00:00"}"#;

fn ahead(seconds: i64) -> DateTime<Utc> {
    let offset = TimeDelta::try_seconds(seconds).expect("the offset fits");
    Utc::now()
        .checked_add_signed(offset)
        .expect("the moment fits")
}

fn expiring_at(moment: DateTime<Utc>, refresh: bool) -> OAuthCredentialPayload {
    OAuthCredentialPayload {
        access_token: Some("tok".to_owned()),
        credential_type: CredentialType::Oauth2,
        refresh_token: refresh.then_some("ref".to_owned()),
        expires_at: Some(moment.to_rfc3339_opts(SecondsFormat::Secs, false)),
        connected_at: None,
        scopes: None,
        identity_binding_proof: None,
        server_id: None,
    }
}

#[test]
fn expiry_treats_a_past_expiry_as_expired() {
    assert!(is_oauth_credential_expired(&payload(PAST), 0));
}

#[test]
fn expiry_treats_a_future_expiry_as_live() {
    assert!(!is_oauth_credential_expired(&payload(FUTURE), 0));
}

#[test]
fn expiry_buffer_treats_a_token_expiring_inside_the_buffer_as_expired() {
    let credential = expiring_at(ahead(30), false);
    assert!(is_oauth_credential_expired(&credential, 60));
    assert!(!is_oauth_credential_expired(&credential, 0));
}

#[test]
fn expiry_buffer_leaves_a_token_expiring_after_the_buffer_live() {
    let credential = expiring_at(ahead(300), false);
    assert!(!is_oauth_credential_expired(&credential, 60));
}

#[test]
fn expiry_buffer_leaves_a_token_expiring_exactly_at_the_deadline_live() {
    let moment = DateTime::parse_from_rfc3339("2024-01-02T03:04:05+00:00")
        .expect("a fixed moment")
        .with_timezone(&Utc);
    let expires = moment + TimeDelta::try_seconds(60).expect("the lifetime fits");
    assert!(!super::super::expires_within_buffer(moment, expires, 60));
    let just_inside = expires - TimeDelta::try_seconds(1).expect("the lifetime fits");
    assert!(super::super::expires_within_buffer(moment, just_inside, 60));
}

#[test]
fn expiry_buffer_can_be_negative_and_keeps_a_past_token_live() {
    assert!(!is_oauth_credential_expired(
        &payload(PAST),
        -10_000_000_000
    ));
}

#[test]
fn expiry_absent_expires_at_is_not_expired() {
    let bare = payload(r#"{"type":"oauth2","access_token":"tok"}"#);
    assert!(!is_oauth_credential_expired(&bare, 0));
    let null = payload(r#"{"type":"oauth2","access_token":"tok","expires_at":null}"#);
    assert!(!is_oauth_credential_expired(&null, 0));
}

#[test]
fn expiry_unparseable_expires_at_is_not_expired() {
    let bad = payload(r#"{"type":"oauth2","access_token":"tok","expires_at":"not-a-date"}"#);
    assert!(!is_oauth_credential_expired(&bad, 0));
}

#[test]
fn expiry_reads_a_naive_timestamp_as_utc() {
    let naive =
        payload(r#"{"type":"oauth2","access_token":"tok","expires_at":"2020-01-01T00:00:00"}"#);
    assert!(is_oauth_credential_expired(&naive, 0));
}

#[test]
fn expiry_reads_the_offset_and_separator_forms_python_reads() {
    let compact = payload(
        r#"{"type":"oauth2","access_token":"tok","expires_at":"2020-01-01T00:00:00+0000"}"#,
    );
    assert!(is_oauth_credential_expired(&compact, 0));
    let spaced = payload(
        r#"{"type":"oauth2","access_token":"tok","expires_at":"2020-01-01 00:00:00+00:00"}"#,
    );
    assert!(is_oauth_credential_expired(&spaced, 0));
    let spaced_zulu =
        payload(r#"{"type":"oauth2","access_token":"tok","expires_at":"2020-01-01 00:00:00Z"}"#);
    assert!(is_oauth_credential_expired(&spaced_zulu, 0));
}

#[test]
fn expiry_does_not_read_python_forms_outside_the_isoformat_grammar() {
    let basic =
        payload(r#"{"type":"oauth2","access_token":"tok","expires_at":"20200101T000000+0000"}"#);
    assert!(!is_oauth_credential_expired(&basic, 0));
    let comma = payload(
        r#"{"type":"oauth2","access_token":"tok","expires_at":"2020-01-02T00:00:00,500+00:00"}"#,
    );
    assert!(!is_oauth_credential_expired(&comma, 0));
}

#[test]
fn expiry_reads_a_date_alone_as_midnight_utc() {
    let past_date = payload(r#"{"type":"oauth2","access_token":"tok","expires_at":"2020-01-01"}"#);
    assert!(is_oauth_credential_expired(&past_date, 0));
    let future_date =
        payload(r#"{"type":"oauth2","access_token":"tok","expires_at":"2100-01-01"}"#);
    assert!(!is_oauth_credential_expired(&future_date, 0));
}

#[test]
fn expiry_ignores_a_buffer_beyond_the_calendar_range() {
    let past = payload(PAST);
    assert!(!is_oauth_credential_expired(&past, i64::MAX));
    assert!(!is_oauth_credential_expired(&past, 9_000_000_000_000_000));
}

#[test]
fn grant_valid_for_unexpired_token() {
    let credential = payload(FUTURE);
    assert_eq!(oauth_grant_state(Some(&credential)), OAuthGrantState::Valid);
}

#[test]
fn grant_refreshable_for_expired_token_with_refresh() {
    let credential = payload(
        r#"{"type":"oauth2","access_token":"tok","refresh_token":"ref","expires_at":"2020-01-01T00:00:00+00:00"}"#,
    );
    assert_eq!(
        oauth_grant_state(Some(&credential)),
        OAuthGrantState::Refreshable
    );
}

#[test]
fn grant_absent_without_access_token() {
    let bare = payload(r#"{"type":"oauth2","expires_at":"2100-01-01T00:00:00+00:00"}"#);
    assert_eq!(oauth_grant_state(Some(&bare)), OAuthGrantState::Absent);
    assert_eq!(oauth_grant_state(None), OAuthGrantState::Absent);
}

#[test]
fn grant_absent_for_an_expired_token_without_refresh() {
    let expired = payload(PAST);
    assert_eq!(oauth_grant_state(Some(&expired)), OAuthGrantState::Absent);
}

#[test]
fn grant_uses_the_refresh_buffer_from_the_constant() {
    assert_eq!(MCP_PER_USER_TOKEN_EXPIRY_BUFFER_SECONDS, 60);
    let inside = expiring_at(ahead(30), true);
    assert_eq!(
        oauth_grant_state(Some(&inside)),
        OAuthGrantState::Refreshable
    );
    let inside_without_refresh = expiring_at(ahead(30), false);
    assert_eq!(
        oauth_grant_state(Some(&inside_without_refresh)),
        OAuthGrantState::Absent
    );
    let beyond = expiring_at(ahead(300), false);
    assert_eq!(oauth_grant_state(Some(&beyond)), OAuthGrantState::Valid);
}

#[test]
fn grant_treats_an_empty_access_token_as_present() {
    let empty =
        payload(r#"{"type":"oauth2","access_token":"","expires_at":"2100-01-01T00:00:00+00:00"}"#);
    assert_eq!(oauth_grant_state(Some(&empty)), OAuthGrantState::Valid);
}
