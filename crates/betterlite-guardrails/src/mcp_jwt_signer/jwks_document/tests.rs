#![cfg(test)]

use std::collections::BTreeMap;

use aws_lc_rs::encoding::AsDer as _;
use aws_lc_rs::rsa::{KeyPair, KeySize};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use betterlite_prompts::arguments::json::{Json, Object};
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use secrecy::ExposeSecret as _;

use super::{get_jwks, jwks_max_age};
use crate::mcp_jwt_signer::signing_key::{SIGNING_KEY_ENV, SigningKey};

fn generated_key() -> SigningKey {
    SigningKey::load(&BTreeMap::new()).expect("generated key")
}

fn persistent_key() -> SigningKey {
    let pair = KeyPair::generate(KeySize::Rsa2048).expect("generated key");
    let der = pair.as_der().expect("pkcs8");
    let pem = pem::encode(&pem::Pem::new("PRIVATE KEY", der.as_ref()));
    let environment = BTreeMap::from([(SIGNING_KEY_ENV.to_owned(), pem)]);
    SigningKey::load(&environment).expect("loaded key")
}

#[test]
fn the_document_holds_one_rs256_signature_key() {
    let key = generated_key();
    let document = get_jwks(&key);
    assert_eq!(document.keys.len(), 1);
    let entry = &document.keys[0];
    assert_eq!(entry.kty, "RSA");
    assert_eq!(entry.alg, "RS256");
    assert_eq!(entry.usage, "sig");
    assert_eq!(entry.kid, key.kid);
    assert_eq!(entry.exponent, "AQAB");
}

#[test]
fn the_document_serializes_with_the_use_field_spelled_like_the_jwk_spec() {
    let key = generated_key();
    let document = get_jwks(&key);
    let json = serde_json::to_string(&document).expect("json");
    let expected = format!(
        r#"{{"keys":[{{"kty":"RSA","alg":"RS256","use":"sig","kid":"{}","n":"{}","e":"{}"}}]}}"#,
        key.kid, document.keys[0].modulus, document.keys[0].exponent
    );
    assert_eq!(json, expected);
}

#[test]
fn the_modulus_and_exponent_are_unpadded_base64url_of_the_minimal_bytes() {
    let key = generated_key();
    let entry = &get_jwks(&key).keys[0];
    assert!(!entry.modulus.contains('='));
    assert!(!entry.exponent.contains('='));
    assert_eq!(
        URL_SAFE_NO_PAD.decode(&entry.modulus).expect("modulus"),
        key.modulus
    );
    assert_eq!(
        URL_SAFE_NO_PAD.decode(&entry.exponent).expect("exponent"),
        key.exponent
    );
    assert_eq!(key.modulus.len(), 256);
}

#[test]
fn a_token_signed_by_the_signer_verifies_against_the_published_key() {
    let key = generated_key();
    let claims = Object::from([
        ("sub".to_owned(), Json::Text("alice".to_owned())),
        ("iss".to_owned(), Json::Text("litellm".to_owned())),
    ]);
    let token = key.sign(&claims).expect("signed token");
    let entry = &get_jwks(&key).keys[0];
    let decoding =
        DecodingKey::from_rsa_components(&entry.modulus, &entry.exponent).expect("decoding key");
    let mut validation = Validation::new(Algorithm::RS256);
    validation.required_spec_claims.clear();
    validation.validate_aud = false;
    let decoded = jsonwebtoken::decode::<Object>(token.expose_secret(), &decoding, &validation)
        .expect("verified token")
        .claims;
    assert_eq!(decoded, claims);
}

#[test]
fn a_persistent_key_is_cached_for_an_hour() {
    assert_eq!(jwks_max_age(&persistent_key()), 3600);
}

#[test]
fn a_generated_key_is_cached_for_five_minutes() {
    assert_eq!(jwks_max_age(&generated_key()), 300);
}
