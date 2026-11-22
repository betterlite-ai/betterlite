#![cfg(test)]

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use jsonwebtoken::errors::{Error, ErrorKind};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use secrecy::{ExposeSecret as _, SecretString};

use super::{
    SESSION_TOKEN_TTL_SECONDS, SESSION_TOKEN_TYPE, SessionClaims, SessionTokenError,
    issue_session_token,
};

const MASTER: &str = "sk-master-key";
const ISSUED_AT: f64 = 1_761_000_000.75;

fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

fn decode_claims(token: &str, key: &SecretString) -> Result<SessionClaims, Error> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = false;
    decode::<SessionClaims>(
        token,
        &DecodingKey::from_secret(key.expose_secret().as_bytes()),
        &validation,
    )
    .map(|data| data.claims)
}

fn segment(token: &str, index: usize) -> String {
    let encoded = token.split('.').nth(index).unwrap();
    let bytes = URL_SAFE_NO_PAD.decode(encoded).unwrap();
    String::from_utf8(bytes).unwrap()
}

#[test]
fn signs_the_claims_litellm_writes() {
    let token = issue_session_token("user-1", "srv-1", &secret(MASTER), ISSUED_AT).unwrap();
    let claims = decode_claims(&token, &secret(MASTER)).unwrap();
    assert_eq!(claims.user_id, "user-1");
    assert_eq!(claims.server_id, "srv-1");
    assert_eq!(claims.token_type, SESSION_TOKEN_TYPE);
}

#[test]
fn truncates_the_issue_time_to_whole_seconds() {
    let token = issue_session_token("user-1", "srv-1", &secret(MASTER), ISSUED_AT).unwrap();
    let claims = decode_claims(&token, &secret(MASTER)).unwrap();
    assert_eq!(claims.iat, 1_761_000_000);
    assert_eq!(
        claims.exp,
        1_761_000_000_u64.saturating_add(SESSION_TOKEN_TTL_SECONDS)
    );
}

#[test]
fn writes_the_payload_keys_in_litellm_order() {
    let token = issue_session_token("user-1", "srv-1", &secret(MASTER), ISSUED_AT).unwrap();
    assert_eq!(
        segment(&token, 1),
        concat!(
            r#"{"user_id":"user-1","server_id":"srv-1","type":"byok_session","#,
            r#""iat":1761000000,"exp":1761003600}"#,
        )
    );
}

#[test]
fn writes_the_header_with_typ_before_alg() {
    let token = issue_session_token("user-1", "srv-1", &secret(MASTER), ISSUED_AT).unwrap();
    assert_eq!(segment(&token, 0), r#"{"typ":"JWT","alg":"HS256"}"#);
}

#[test]
fn fails_to_verify_with_another_master_key() {
    let token = issue_session_token("user-1", "srv-1", &secret(MASTER), ISSUED_AT).unwrap();
    let error = decode_claims(&token, &secret("sk-other-key")).unwrap_err();
    assert!(matches!(error.kind(), ErrorKind::InvalidSignature));
}

#[test]
fn rejects_a_negative_issue_time() {
    let error = issue_session_token("user-1", "srv-1", &secret(MASTER), -1.0).unwrap_err();
    assert!(matches!(error, SessionTokenError::IssuedAt(_)));
}

#[test]
fn rejects_an_infinite_issue_time() {
    let error = issue_session_token("user-1", "srv-1", &secret(MASTER), f64::INFINITY).unwrap_err();
    assert!(matches!(error, SessionTokenError::IssuedAt(_)));
}
