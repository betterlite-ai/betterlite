use crate::session_tokens::{
    is_session_refresh_token, is_session_token, mint_session_refresh_token, mint_session_token,
    open_session_refresh_token, open_session_token,
};
use crate::types::{SessionKeys, SessionPrincipal, SessionSigningKeys};
use chrono::{TimeZone as _, Utc};
use secrecy::{ExposeSecret as _, SecretString};
fn fixture() -> (SessionSigningKeys, SessionPrincipal, chrono::DateTime<Utc>) {
    let keys = SessionSigningKeys::Symmetric(SessionKeys {
        signing_key: SecretString::from("0123456789abcdef0123456789abcdef"),
    });
    let principal = SessionPrincipal {
        user_id: "user-1".to_owned(),
        client_id: "client-1".to_owned(),
        resource_server_id: Some("server-1".to_owned()),
        audience: None,
        team_id: Some("team-1".to_owned()),
    };
    let now = Utc
        .with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
        .single()
        .expect("valid fixture clock");
    (keys, principal, now)
}
#[test]
fn session_token_tests_behavior() {
    let (keys, principal, now) = fixture();
    assert!(is_session_token("llm_session_anything"));
    assert!(is_session_refresh_token("llm_srefresh_anything"));
    let access = mint_session_token(&principal, &keys, now).expect("access mint");
    let opened = open_session_token(access.token.expose_secret(), &keys, now).expect("access open");
    assert_eq!(opened.principal, principal);
    assert_eq!(opened.exp - opened.iat, 3_600);
    let refresh = mint_session_refresh_token(&principal, &keys, now).expect("refresh mint");
    open_session_refresh_token(refresh.token.expose_secret(), &keys, now).unwrap();
    open_session_token(refresh.token.expose_secret(), &keys, now).unwrap_err();
    open_session_token("llm_session_not.a.jwt", &keys, now).unwrap_err();
}
