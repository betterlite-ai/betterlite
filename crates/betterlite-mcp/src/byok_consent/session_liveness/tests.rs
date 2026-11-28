#![cfg(test)]

use crate::byok_consent::session::authenticate_session;
use crate::byok_consent::test_support::{Claims, Keys, MASTER_KEY, master, now, token, ui_claims};

async fn admits(claims: Claims<'_>, resolves: bool) -> Option<String> {
    let keys = Keys { resolves };
    let master_key = master(MASTER_KEY);
    let header = token(claims, MASTER_KEY);
    authenticate_session(Some(&header), Some(&master_key), Some(&keys), now())
        .await
        .ok()
}

#[tokio::test]
async fn a_cookie_without_a_session_key_is_not_re_resolved() {
    assert_eq!(admits(ui_claims(), false).await, Some("user-1".to_owned()));
}

#[tokio::test]
async fn a_non_sk_session_key_passes_even_when_the_lookup_fails() {
    let claims = Claims {
        key: Some("blob-session-token"),
        ..ui_claims()
    };
    assert_eq!(admits(claims, false).await, Some("user-1".to_owned()));
}

#[tokio::test]
async fn a_live_sk_session_key_admits_the_user() {
    let claims = Claims {
        key: Some("sk-live-key"),
        ..ui_claims()
    };
    assert_eq!(admits(claims, true).await, Some("user-1".to_owned()));
}

#[tokio::test]
async fn a_revoked_sk_session_key_is_refused() {
    let claims = Claims {
        key: Some("sk-revoked-key"),
        ..ui_claims()
    };
    assert_eq!(admits(claims, false).await, None);
}

#[tokio::test]
async fn an_sk_session_key_passes_when_no_lookup_is_configured() {
    let claims = Claims {
        key: Some("sk-unknown-key"),
        ..ui_claims()
    };
    let master_key = master(MASTER_KEY);
    let header = token(claims, MASTER_KEY);
    let admitted = authenticate_session(Some(&header), Some(&master_key), None, now()).await;
    assert_eq!(admitted.ok(), Some("user-1".to_owned()));
}
