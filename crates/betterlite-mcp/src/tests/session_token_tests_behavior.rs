use crate::session_tokens::{
    MAX_SESSION_TOKEN_BYTES, SESSION_REFRESH_PREFIX, SESSION_TOKEN_PREFIX,
    is_session_refresh_token, is_session_token, mint_session_refresh_token, mint_session_token,
    open_session_refresh_token, open_session_token,
};
use crate::types::{SessionKeys, SessionPrincipal, SessionSigningKeys};
use chrono::{TimeZone as _, Utc};
use secrecy::{ExposeSecret as _, SecretString};

fn keys() -> SessionSigningKeys {
    SessionSigningKeys::Symmetric(SessionKeys {
        signing_key: SecretString::from("0123456789abcdef0123456789abcdef"),
    })
}
fn principal(client_id: &str) -> SessionPrincipal {
    SessionPrincipal {
        user_id: "user-1".to_owned(),
        client_id: client_id.to_owned(),
        resource_server_id: Some("server-1".to_owned()),
        audience: None,
        team_id: Some("team-1".to_owned()),
    }
}

#[test]
fn session_token_tests_behavior_symmetric_access() {
    let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let minted = mint_session_token(&principal("client-1"), &keys(), now).unwrap();
    assert!(
        minted
            .token
            .expose_secret()
            .starts_with(SESSION_TOKEN_PREFIX)
    );
    let opened = open_session_token(minted.token.expose_secret(), &keys(), now).unwrap();
    assert_eq!(opened.principal, principal("client-1"));
    assert_eq!(opened.exp - opened.iat, 3_600);
}

#[test]
fn session_token_tests_behavior_refresh_kind_and_rotation() {
    let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let minted = mint_session_refresh_token(&principal("client-1"), &keys(), now).unwrap();
    assert!(
        minted
            .token
            .expose_secret()
            .starts_with(SESSION_REFRESH_PREFIX)
    );
    open_session_refresh_token(minted.token.expose_secret(), &keys(), now).unwrap();
    open_session_token(minted.token.expose_secret(), &keys(), now).unwrap_err();
}

#[test]
fn session_token_tests_behavior_prefix_and_hostile_input() {
    assert!(is_session_token("llm_session_anything"));
    assert!(!is_session_token("llm_srefresh_anything"));
    assert!(is_session_refresh_token("llm_srefresh_anything"));
    assert!(!is_session_refresh_token("llm_session_anything"));
    let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    open_session_token("llm_session_not.a.jwt", &keys(), now).unwrap_err();
    open_session_token(
        &format!("llm_session_{}", "x".repeat(MAX_SESSION_TOKEN_BYTES)),
        &keys(),
        now,
    )
    .unwrap_err();
}

#[test]
fn session_token_tests_behavior_expiry_and_size() {
    let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let expired_at = now + chrono::Duration::seconds(3_600);
    let minted = mint_session_token(&principal("client-1"), &keys(), now).unwrap();
    open_session_token(minted.token.expose_secret(), &keys(), expired_at).unwrap_err();
    let mut client_id = "client-".to_owned();
    client_id.push_str(&"x".repeat(MAX_SESSION_TOKEN_BYTES));
    let oversized = principal(&client_id);
    mint_session_token(&oversized, &keys(), now).unwrap_err();
}
