#![cfg(test)]

use rstest::rstest;

use crate::byok_consent::session::authenticate_session;
use crate::byok_consent::test_support::{
    Claims, FUTURE_EXPIRY, Keys, MASTER_KEY, master, now, token, ui_claims,
};
use crate::session_admission::auth::SessionAuthError;

async fn admits(
    cookie_header: Option<&str>,
    master_key: Option<&str>,
    resolves: bool,
) -> Option<String> {
    let keys = Keys { resolves };
    let secret = master_key.map(master);
    authenticate_session(cookie_header, secret.as_ref(), Some(&keys), now())
        .await
        .ok()
}

#[tokio::test]
async fn a_missing_cookie_is_refused() {
    assert_eq!(admits(None, Some(MASTER_KEY), true).await, None);
}

#[tokio::test]
async fn a_cookie_that_is_not_a_jwt_is_refused() {
    assert_eq!(
        admits(Some("token=not-a-jwt"), Some(MASTER_KEY), true).await,
        None
    );
}

#[tokio::test]
async fn a_cookie_signed_with_another_key_is_refused() {
    let header = token(ui_claims(), "sk-other-master");
    assert_eq!(admits(Some(&header), Some(MASTER_KEY), true).await, None);
}

#[rstest]
#[case::missing_exp(Claims { exp: None, ..ui_claims() })]
#[case::expired(Claims { exp: Some(0), ..ui_claims() })]
#[case::byok_session_token(Claims { token_type: Some("byok_session"), ..ui_claims() })]
#[case::missing_login_method(Claims { login_method: None, ..ui_claims() })]
#[case::unknown_login_method(Claims { login_method: Some("api_key"), ..ui_claims() })]
#[case::missing_user_id(Claims { user_id: None, ..ui_claims() })]
#[case::empty_user_id(Claims { user_id: Some(""), ..ui_claims() })]
#[tokio::test]
async fn a_cookie_that_is_not_a_ui_session_is_refused(#[case] claims: Claims<'static>) {
    let header = token(claims, MASTER_KEY);
    assert_eq!(admits(Some(&header), Some(MASTER_KEY), true).await, None);
}

#[tokio::test]
async fn an_sso_session_admits_the_cookie_user() {
    let header = token(ui_claims(), MASTER_KEY);
    assert_eq!(
        admits(Some(&header), Some(MASTER_KEY), true).await,
        Some("user-1".to_owned())
    );
}

#[tokio::test]
async fn a_username_password_session_admits_the_cookie_user() {
    let claims = Claims {
        login_method: Some("username_password"),
        ..ui_claims()
    };
    let header = token(claims, MASTER_KEY);
    assert_eq!(
        admits(Some(&header), Some(MASTER_KEY), true).await,
        Some("user-1".to_owned())
    );
}

#[tokio::test]
async fn a_proxy_without_a_master_key_refuses_the_login() {
    let header = token(ui_claims(), MASTER_KEY);
    assert_eq!(admits(Some(&header), None, true).await, None);
}

#[tokio::test]
async fn a_refused_login_is_login_required() {
    let keys = Keys { resolves: true };
    let master_key = master(MASTER_KEY);
    let result = authenticate_session(None, Some(&master_key), Some(&keys), now()).await;
    assert!(matches!(result, Err(SessionAuthError::LoginRequired)));
}

#[tokio::test]
async fn an_expiry_in_the_future_is_not_a_failure() {
    let claims = Claims {
        exp: Some(FUTURE_EXPIRY),
        ..ui_claims()
    };
    let header = token(claims, MASTER_KEY);
    assert_eq!(
        admits(Some(&header), Some(MASTER_KEY), true).await,
        Some("user-1".to_owned())
    );
}
