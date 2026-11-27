#![cfg(test)]

use std::collections::BTreeMap;

use async_trait::async_trait;
use aws_lc_rs::hmac;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use betterlite_auth::auth_key_lookup::{
    KeyAuthenticator, KeyLookupError, KeyLookupOutcome, key_identity,
};
use betterlite_auth::resolved_key::auth::{AuthError, AuthInput, UserAPIKeyAuth};
use chrono::{DateTime, Utc};
use http::StatusCode;
use serde::{Deserialize, Serialize};

use super::auth::{LOGIN_REQUIRED, SessionAuthError, byok_session_auth};
use super::cookie::session_token_cookie;
use super::identity::session_identity_from_cookie;

const MASTER_KEY: &str = "sk-oracle-master";
const NOW: i64 = 1_700_000_000;
const FUTURE_EXPIRY: i64 = 4_102_444_800;

#[derive(Clone, Copy, Default, Serialize)]
struct Claims<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    exp: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    iat: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    login_method: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    key: Option<&'a str>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    token_type: Option<&'a str>,
}

fn signed(claims: Claims<'_>, master_key: &str) -> String {
    let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"HS256","typ":"JWT"}"#);
    let payload = URL_SAFE_NO_PAD.encode(serde_json::to_string(&claims).unwrap());
    let signing_input = format!("{header}.{payload}");
    let signature = hmac::sign(
        &hmac::Key::new(hmac::HMAC_SHA256, master_key.as_bytes()),
        signing_input.as_bytes(),
    );
    format!(
        "{signing_input}.{}",
        URL_SAFE_NO_PAD.encode(signature.as_ref())
    )
}

fn token(claims: Claims<'_>, master_key: &str) -> String {
    format!("token={}", signed(claims, master_key))
}

struct Keys {
    resolves: bool,
}

#[async_trait]
impl KeyAuthenticator for Keys {
    async fn authenticate(&self, presented: &str) -> Result<KeyLookupOutcome, KeyLookupError> {
        if !self.resolves {
            return Err(KeyLookupError::MissingKey {
                key: presented.to_owned(),
            });
        }
        let auth = UserAPIKeyAuth::construct(AuthInput::Dict(BTreeMap::new())).unwrap();
        Ok(KeyLookupOutcome {
            auth,
            hashed_token: presented.to_owned(),
        })
    }
}

async fn admits(cookie_header: Option<&str>, master_key: &str, resolves: bool) -> Option<String> {
    let keys = Keys { resolves };
    let now = DateTime::from_timestamp(NOW, 0).unwrap();
    match byok_session_auth(cookie_header, master_key, Some(&keys), now).await {
        Ok(auth) => key_identity(&auth).user_id,
        Err(SessionAuthError::LoginRequired) => None,
        Err(error) => panic!("the session principal did not construct: {error}"),
    }
}

fn ui_claims() -> Claims<'static> {
    Claims {
        exp: Some(FUTURE_EXPIRY),
        iat: None,
        login_method: Some("sso"),
        user_id: Some("user-1"),
        key: None,
        token_type: None,
    }
}

#[tokio::test]
async fn cookie_missing() {
    assert_eq!(admits(None, MASTER_KEY, true).await, None);
}

#[tokio::test]
async fn cookie_invalid() {
    assert_eq!(
        admits(Some("token=not-a-jwt"), MASTER_KEY, true).await,
        None
    );
}

#[tokio::test]
async fn cookie_wrong_signature() {
    let header = token(ui_claims(), "sk-other-master");
    assert_eq!(admits(Some(&header), MASTER_KEY, true).await, None);
}

#[tokio::test]
async fn cookie_missing_exp() {
    let claims = Claims {
        exp: None,
        iat: None,
        login_method: Some("sso"),
        user_id: Some("user-1"),
        key: None,
        token_type: None,
    };
    assert_eq!(
        admits(Some(&token(claims, MASTER_KEY)), MASTER_KEY, true).await,
        None
    );
}

#[tokio::test]
async fn cookie_expired() {
    let claims = Claims {
        exp: Some(0),
        ..ui_claims()
    };
    assert_eq!(
        admits(Some(&token(claims, MASTER_KEY)), MASTER_KEY, true).await,
        None
    );
}

#[tokio::test]
async fn cookie_wrong_type() {
    let claims = Claims {
        iat: Some(NOW),
        token_type: Some("byok_session"),
        ..ui_claims()
    };
    assert_eq!(
        admits(Some(&token(claims, MASTER_KEY)), MASTER_KEY, true).await,
        None
    );
}

#[tokio::test]
async fn cookie_missing_login_method() {
    let claims = Claims {
        login_method: None,
        user_id: Some("user-1"),
        ..ui_claims()
    };
    assert_eq!(
        admits(Some(&token(claims, MASTER_KEY)), MASTER_KEY, true).await,
        None
    );
}

#[tokio::test]
async fn cookie_unknown_login_method() {
    let claims = Claims {
        login_method: Some("api_key"),
        ..ui_claims()
    };
    assert_eq!(
        admits(Some(&token(claims, MASTER_KEY)), MASTER_KEY, true).await,
        None
    );
}

#[tokio::test]
async fn cookie_missing_user_id() {
    let claims = Claims {
        user_id: None,
        login_method: Some("sso"),
        ..ui_claims()
    };
    assert_eq!(
        admits(Some(&token(claims, MASTER_KEY)), MASTER_KEY, true).await,
        None
    );
}

#[tokio::test]
async fn cookie_empty_user_id() {
    let claims = Claims {
        user_id: Some(""),
        ..ui_claims()
    };
    assert_eq!(
        admits(Some(&token(claims, MASTER_KEY)), MASTER_KEY, true).await,
        None
    );
}

#[tokio::test]
async fn cookie_with_optional_key() {
    let claims = Claims {
        key: Some("ui_session_blob"),
        ..ui_claims()
    };
    assert_eq!(
        admits(Some(&token(claims, MASTER_KEY)), MASTER_KEY, false).await,
        Some("user-1".to_owned())
    );
}

#[tokio::test]
async fn cookie_blob_token_no_key() {
    let claims = Claims {
        login_method: Some("username_password"),
        user_id: Some("user-2"),
        ..ui_claims()
    };
    assert_eq!(
        admits(Some(&token(claims, MASTER_KEY)), MASTER_KEY, false).await,
        Some("user-2".to_owned())
    );
}

#[tokio::test]
async fn cookie_sk_key_revoked() {
    let claims = Claims {
        key: Some("sk-revoked-key"),
        ..ui_claims()
    };
    assert_eq!(
        admits(Some(&token(claims, MASTER_KEY)), MASTER_KEY, false).await,
        None
    );
}

#[tokio::test]
async fn cookie_valid_with_key() {
    let claims = Claims {
        key: Some("sk-live-key"),
        ..ui_claims()
    };
    assert_eq!(
        admits(Some(&token(claims, MASTER_KEY)), MASTER_KEY, true).await,
        Some("user-1".to_owned())
    );
}

#[tokio::test]
async fn cookie_sk_key_without_database() {
    let claims = Claims {
        key: Some("sk-live-key"),
        ..ui_claims()
    };
    let now = DateTime::from_timestamp(NOW, 0).unwrap();
    let auth = byok_session_auth(Some(&token(claims, MASTER_KEY)), MASTER_KEY, None, now)
        .await
        .unwrap();
    assert_eq!(key_identity(&auth).user_id, Some("user-1".to_owned()));
}

#[tokio::test]
async fn cookie_empty_key_claim() {
    let claims = Claims {
        key: Some(""),
        ..ui_claims()
    };
    let header = token(claims, MASTER_KEY);
    let identity = session_identity_from_cookie(
        Some(&header),
        MASTER_KEY,
        DateTime::from_timestamp(NOW, 0).unwrap(),
    )
    .unwrap();
    assert_eq!(identity.session_key, None);
    assert_eq!(
        admits(Some(&header), MASTER_KEY, false).await,
        Some("user-1".to_owned())
    );
}

#[tokio::test]
async fn cookie_master_key_unset() {
    let header = token(ui_claims(), MASTER_KEY);
    assert_eq!(admits(Some(&header), "", true).await, None);
}

#[tokio::test]
async fn cookie_quoted_value() {
    let header = format!("\"{}\"", signed(ui_claims(), MASTER_KEY));
    assert_eq!(
        admits(Some(&format!("token={header}")), MASTER_KEY, true).await,
        Some("user-1".to_owned())
    );
}

#[tokio::test]
async fn cookie_after_another_cookie() {
    let header = format!("other=1; {}", token(ui_claims(), MASTER_KEY));
    assert_eq!(
        admits(Some(&header), MASTER_KEY, true).await,
        Some("user-1".to_owned())
    );
}

#[tokio::test]
async fn cookie_last_token_wins() {
    let header = format!("token=stale; {}", token(ui_claims(), MASTER_KEY));
    assert_eq!(
        admits(Some(&header), MASTER_KEY, true).await,
        Some("user-1".to_owned())
    );
}

#[test]
fn cookie_parser_reads_the_token_chunk_only() {
    assert_eq!(session_token_cookie(None), None);
    assert_eq!(session_token_cookie(Some("other=1")), None);
    assert_eq!(
        session_token_cookie(Some(" token = value ")),
        Some("value".to_owned())
    );
    assert_eq!(session_token_cookie(Some("token=")), Some(String::new()));
}

#[test]
fn cookie_parser_unquotes_octal_and_escaped_characters() {
    assert_eq!(
        session_token_cookie(Some("token=\"a\\157b\"")),
        Some("aob".to_owned())
    );
    assert_eq!(
        session_token_cookie(Some("token=\"a\\\"b\"")),
        Some("a\"b".to_owned())
    );
    assert_eq!(
        session_token_cookie(Some("token=\"a\\\\b\"")),
        Some("a\\b".to_owned())
    );
    assert_eq!(
        session_token_cookie(Some("token=\"a\\\nb\"")),
        Some("a\\\nb".to_owned())
    );
    assert_eq!(
        session_token_cookie(Some("token=\"plain\"")),
        Some("plain".to_owned())
    );
}

#[derive(Deserialize)]
struct PrincipalSnapshot {
    api_key: String,
    user_id: String,
}

#[tokio::test]
async fn session_auth_constructs_the_byok_session_principal() {
    let keys = Keys { resolves: true };
    let now = DateTime::from_timestamp(NOW, 0).unwrap();
    let auth = byok_session_auth(
        Some(&token(ui_claims(), MASTER_KEY)),
        MASTER_KEY,
        Some(&keys),
        now,
    )
    .await
    .unwrap();
    let snapshot: PrincipalSnapshot =
        serde_json::from_value(serde_json::to_value(auth).unwrap()).unwrap();
    assert_eq!(snapshot.api_key, "byok_session_cookie");
    assert_eq!(snapshot.user_id, "user-1");
}

#[test]
fn session_auth_errors_carry_the_lite_llm_status() {
    assert_eq!(LOGIN_REQUIRED, "login_required");
    assert_eq!(
        SessionAuthError::LoginRequired.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        SessionAuthError::Construct(AuthError::NonStringApiKey).status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
}
