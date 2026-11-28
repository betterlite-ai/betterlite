use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::jwt_verification::algorithms::JwtAlgorithm;
use crate::jwt_verification::errors::VerificationError;
use crate::jwt_verification::keys::{PublicKey, unsigned, verify_signature};

use super::fixtures;
use super::support::parse_json;

#[test]
fn tampered_ec_signature_is_rejected() {
    let jwk = parse_json(fixtures::P256_JWK);
    let key = PublicKey::Jwk(&jwk);
    let error = verify_signature(&key, JwtAlgorithm::Es256, b"message", "AAAA").unwrap_err();
    assert_eq!(
        error.to_string(),
        "Validation fails: Signature verification failed"
    );
}

#[test]
fn tampered_p521_signature_is_rejected() {
    let jwk = parse_json(fixtures::P521_JWK);
    let key = PublicKey::Jwk(&jwk);
    let error = verify_signature(&key, JwtAlgorithm::Es512, b"message", "AAAA").unwrap_err();
    assert_eq!(
        error.to_string(),
        "Validation fails: Signature verification failed"
    );
}

#[test]
fn an_invalid_p521_point_is_an_unusable_key() {
    let mut fields: IndexMap<String, Json> = serde_json::from_str(fixtures::P521_JWK).unwrap();
    fields.insert("x".to_owned(), Json::Text("A".repeat(88)));
    fields.insert("y".to_owned(), Json::Text("A".repeat(88)));
    let jwk = Json::Object(fields);
    let key = PublicKey::Jwk(&jwk);
    let error = verify_signature(&key, JwtAlgorithm::Es512, b"message", "AAAA").unwrap_err();
    let VerificationError::Validation { detail } = error else {
        panic!("an invalid P-521 point is not usable");
    };
    assert!(
        detail.starts_with("the configured key is not usable: "),
        "{detail}"
    );
}

#[test]
fn tampered_certificate_signature_is_rejected() {
    let key = PublicKey::Certificate(fixtures::RSA_CERTIFICATE);
    let error = verify_signature(&key, JwtAlgorithm::Rs256, b"message", "AAAA").unwrap_err();
    assert_eq!(
        error.to_string(),
        "Validation fails: Signature verification failed"
    );
}

#[test]
fn ed_certificate_checks_its_signature() {
    let key = PublicKey::Certificate(fixtures::ED_CERTIFICATE);
    let error = verify_signature(&key, JwtAlgorithm::EdDsa, b"message", "AAAA").unwrap_err();
    assert_eq!(
        error.to_string(),
        "Validation fails: Signature verification failed"
    );
}

#[test]
fn ed_certificate_rejects_a_non_ed_algorithm() {
    let key = PublicKey::Certificate(fixtures::ED_CERTIFICATE);
    let error = verify_signature(&key, JwtAlgorithm::Rs256, b"message", "AAAA").unwrap_err();
    assert_eq!(
        error.to_string(),
        "Validation fails: the configured key is not compatible with the token algorithm"
    );
}

#[test]
fn unsigned_strips_only_a_padding_zero() {
    assert_eq!(unsigned(&[0_u8, 1_u8, 2_u8]), [1_u8, 2_u8].as_slice());
    assert_eq!(unsigned(&[1_u8, 2_u8]), [1_u8, 2_u8].as_slice());
    assert_eq!(unsigned(&[0_u8]), [0_u8].as_slice());
}
