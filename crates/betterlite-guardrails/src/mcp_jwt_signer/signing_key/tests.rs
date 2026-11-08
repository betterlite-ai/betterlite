#![cfg(test)]

use std::collections::BTreeMap;
use std::env::temp_dir;
use std::fs;
use std::path::PathBuf;

use aws_lc_rs::encoding::AsDer as _;
use aws_lc_rs::rsa::{KeyPair, KeySize};
use aws_lc_rs::signature::KeyPair as _;
use betterlite_prompts::arguments::json::{Json, Object};
use jsonwebtoken::Algorithm;
use secrecy::ExposeSecret as _;
use sha2::{Digest as _, Sha256};

use super::{SIGNING_KEY_ENV, SigningKey, SigningKeyError};

fn environment(material: Option<&str>) -> BTreeMap<String, String> {
    material
        .map(|value| (SIGNING_KEY_ENV.to_owned(), value.to_owned()))
        .into_iter()
        .collect()
}

fn key_pair() -> KeyPair {
    KeyPair::generate(KeySize::Rsa2048).expect("generated key")
}

fn pkcs8_pem(key_pair: &KeyPair) -> String {
    let der = key_pair.as_der().expect("pkcs8");
    pem::encode(&pem::Pem::new("PRIVATE KEY", der.as_ref()))
}

fn pkcs1_pem(key_pair: &KeyPair) -> String {
    let der = key_pair.as_der().expect("pkcs8");
    let info = pkcs8::PrivateKeyInfoRef::try_from(der.as_ref()).expect("private key info");
    pem::encode(&pem::Pem::new(
        "RSA PRIVATE KEY",
        info.private_key.as_bytes(),
    ))
}

fn temp_path() -> PathBuf {
    temp_dir().join(format!(
        "betterlite-signing-key-{}.pem",
        uuid::Uuid::new_v4()
    ))
}

fn expected_kid(key_pair: &KeyPair) -> String {
    let spki = key_pair.public_key().as_der().expect("spki");
    hex::encode(Sha256::digest(spki.as_ref()))
        .chars()
        .take(16)
        .collect()
}

#[test]
fn a_pkcs8_pem_in_the_environment_is_a_persistent_key() {
    let pair = key_pair();
    let key = SigningKey::load(&environment(Some(&pkcs8_pem(&pair)))).expect("loaded key");
    assert!(key.persistent);
    assert_eq!(key.kid, expected_kid(&pair));
}

#[test]
fn a_pkcs1_pem_in_the_environment_uses_the_same_kid() {
    let pair = key_pair();
    let key = SigningKey::load(&environment(Some(&pkcs1_pem(&pair)))).expect("loaded key");
    assert!(key.persistent);
    assert_eq!(key.kid, expected_kid(&pair));
}

#[test]
fn the_kid_is_the_first_sixteen_hex_characters_of_the_spki_digest() {
    let pair = key_pair();
    let digest = expected_kid(&pair);
    let key = SigningKey::load(&environment(Some(&pkcs8_pem(&pair)))).expect("loaded key");
    assert_eq!(key.kid, digest);
    assert_eq!(key.kid.chars().count(), 16);
    assert!(
        key.kid
            .chars()
            .all(|character| character.is_ascii_hexdigit() && !character.is_ascii_uppercase())
    );
}

#[test]
fn a_file_uri_reads_the_key_from_disk() {
    let pair = key_pair();
    let path = temp_path();
    fs::write(&path, pkcs8_pem(&pair)).expect("written key");
    let material = format!("file://{}", path.display());
    let key = SigningKey::load(&environment(Some(&material))).expect("loaded key");
    fs::remove_file(&path).expect("removed key");
    assert!(key.persistent);
    assert_eq!(key.kid, expected_kid(&pair));
}

#[test]
fn a_file_uri_that_cannot_be_read_is_an_error() {
    let path = temp_path();
    let material = format!("file://{}", path.display());
    let error = SigningKey::load(&environment(Some(&material))).expect_err("missing file");
    assert!(matches!(error, SigningKeyError::ReadFile { .. }));
    assert!(error.to_string().contains(&path.display().to_string()));
}

#[test]
fn a_file_that_does_not_hold_a_pem_is_an_error() {
    let path = temp_path();
    fs::write(&path, "not a pem document").expect("written key");
    let material = format!("file://{}", path.display());
    let error = SigningKey::load(&environment(Some(&material))).expect_err("bad pem");
    fs::remove_file(&path).expect("removed key");
    assert!(matches!(error, SigningKeyError::Pem { .. }));
}

#[test]
fn a_bad_pem_is_an_error_and_not_a_generated_key() {
    let error = SigningKey::load(&environment(Some("not a pem document")))
        .expect_err("bad pem is an error");
    assert!(matches!(error, SigningKeyError::Pem { .. }));
}

#[test]
fn a_pem_that_does_not_hold_a_private_key_is_an_error() {
    let material = pem::encode(&pem::Pem::new("PRIVATE KEY", [1_u8, 2, 3]));
    let error = SigningKey::load(&environment(Some(&material))).expect_err("bad pkcs8");
    assert!(matches!(error, SigningKeyError::Pkcs8 { .. }));
}

#[test]
fn an_unset_variable_generates_a_key_that_is_not_persistent() {
    let key = SigningKey::load(&environment(None)).expect("generated key");
    assert!(!key.persistent);
    assert_eq!(key.kid.chars().count(), 16);
}

#[test]
fn an_empty_variable_generates_a_key() {
    let key = SigningKey::load(&environment(Some(""))).expect("generated key");
    assert!(!key.persistent);
}

#[test]
fn a_generated_key_has_the_rsa_2048_modulus_and_exponent_65537() {
    let key = SigningKey::load(&environment(None)).expect("generated key");
    assert_eq!(key.modulus.len(), 256);
    assert_eq!(key.exponent, [1, 0, 1]);
    assert_ne!(key.modulus.first(), Some(&0));
}

#[test]
fn signing_puts_the_kid_and_rs256_in_the_header() {
    let pair = key_pair();
    let key = SigningKey::load(&environment(Some(&pkcs8_pem(&pair)))).expect("loaded key");
    let claims = Object::from([("sub".to_owned(), Json::Text("alice".to_owned()))]);
    let token = key.sign(&claims).expect("signed token");
    let header = jsonwebtoken::decode_header(token.expose_secret()).expect("header");
    assert_eq!(header.alg, Algorithm::RS256);
    assert_eq!(header.kid.as_deref(), Some(key.kid.as_str()));
}
