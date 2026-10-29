#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_secrets::secret_codec::Algorithm;
use secrecy::ExposeSecret as _;

use crate::loader::ConfigLoader;

fn load_cipher(salt: Option<&str>, yaml: &str) -> (Option<String>, Algorithm) {
    let lookup: BTreeMap<String, String> = salt
        .map(|value| ("LITELLM_SALT_KEY".to_owned(), value.to_owned()))
        .into_iter()
        .collect();
    let config = ConfigLoader { lookup }.load(yaml).unwrap();
    let cipher = config.general_settings.cipher;
    (
        cipher.signing_key.map(|key| key.expose_secret().to_owned()),
        cipher.algorithm,
    )
}

#[test]
fn loads_the_salt_key_before_the_master_key() {
    let (key, _) = load_cipher(Some("salt-one"), "general_settings: {master_key: other}");
    assert_eq!(key, Some("salt-one".to_owned()));
}

#[test]
fn loads_the_master_key_when_the_salt_key_is_absent() {
    let (key, _) = load_cipher(None, "general_settings: {master_key: fallback}");
    assert_eq!(key, Some("fallback".to_owned()));
}

#[test]
fn loads_the_legacy_algorithm_by_default() {
    let (_, algorithm) = load_cipher(Some("salt-one"), "general_settings: {}");
    assert_eq!(algorithm, Algorithm::XSalsa20Poly1305);
}

#[test]
fn loads_the_aes_algorithm_for_a_casefolded_match() {
    let (_, algorithm) = load_cipher(
        Some("salt-one"),
        "general_settings: {encryption_algorithm: AES-256-GCM}",
    );
    assert_eq!(algorithm, Algorithm::Aes256Gcm);
}

#[test]
fn loads_the_legacy_algorithm_for_an_unrecognized_value() {
    let (_, algorithm) = load_cipher(
        Some("salt-one"),
        "general_settings: {encryption_algorithm: chacha20}",
    );
    assert_eq!(algorithm, Algorithm::XSalsa20Poly1305);
}
