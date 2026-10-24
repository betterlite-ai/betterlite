#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_secrets::error::SecretError;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{NonceSource, StoredCipher, StoredValue};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use crate::loader::ConfigLoader;

// LiteLLM's ciphertext of "hello" under each key, with nonce bytes 0, 1, 2, ...
const SALT_ONE: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXccVZjgu1TZy2W9RyeYfnLiSlXcAj";
const SALT_ONE_AES: &str = "v2:gcm:AAECAwQFBgcICQoLu13Sfnjzb3vLfCPO7ClYdksHnW1L";
const EMPTY_SALT: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXpRe20oobZi7AwOoQfhXE0GCNqz9X";
const OVERRIDE: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXfuRp9WUvbNm7fn_PA-d4hvaoT58a";

struct CountingNonces;

impl NonceSource for CountingNonces {
    fn fill(&self, nonce: &mut [u8]) -> Result<(), SecretError> {
        nonce
            .iter_mut()
            .zip(0_u8..)
            .for_each(|(byte, index)| *byte = index);
        Ok(())
    }
}

fn encrypt_hello(salt: Option<&str>, yaml: &str, override_key: Option<&str>) -> String {
    let lookup: BTreeMap<String, String> = salt
        .map(|value| ("LITELLM_SALT_KEY".to_owned(), value.to_owned()))
        .into_iter()
        .collect();
    let config = ConfigLoader { lookup }.load(yaml).unwrap();
    let cipher = StoredCipher {
        settings: config.general_settings.cipher,
        nonces: CountingNonces,
        diagnostics: TracingDiagnostics,
    };
    let key = override_key.map(SecretString::from);
    match cipher.encrypt_value(
        StoredValue::<()>::Text(SecretString::from("hello")),
        key.as_ref(),
    ) {
        Ok(StoredValue::Text(text)) => text.expose_secret().to_owned(),
        Ok(StoredValue::Other(())) => "passed through".to_owned(),
        Err(error) => error.to_string(),
    }
}

#[rstest]
#[case::salt_before_master(
    Some("salt-one"),
    "general_settings: {master_key: other}",
    None,
    SALT_ONE
)]
#[case::absent_salt_uses_master(None, "general_settings: {master_key: salt-one}", None, SALT_ONE)]
#[case::empty_salt_kept(Some(""), "general_settings: {master_key: salt-one}", None, EMPTY_SALT)]
#[case::no_key_at_all(
    None,
    "{}",
    None,
    "no LITELLM_SALT_KEY or master key is set to encrypt with"
)]
#[case::truthy_new_key_override(Some("salt-one"), "{}", Some("override-key"), OVERRIDE)]
#[case::empty_new_key_falls_back(Some("salt-one"), "{}", Some(""), SALT_ONE)]
#[case::default_legacy_algorithm(Some("salt-one"), "general_settings: {}", None, SALT_ONE)]
#[case::aes_casefold_without_trim(
    Some("salt-one"),
    "general_settings: {encryption_algorithm: AES-256-GCM}",
    None,
    SALT_ONE_AES
)]
#[case::aes_untrimmed_is_legacy(
    Some("salt-one"),
    "general_settings: {encryption_algorithm: ' aes-256-gcm'}",
    None,
    SALT_ONE
)]
#[case::unknown_algorithm_legacy(
    Some("salt-one"),
    "general_settings: {encryption_algorithm: chacha20}",
    None,
    SALT_ONE
)]
#[case::non_string_algorithm_legacy(
    Some("salt-one"),
    "general_settings: {encryption_algorithm: [aes-256-gcm]}",
    None,
    SALT_ONE
)]
#[case::null_algorithm_legacy(
    Some("salt-one"),
    "general_settings: {encryption_algorithm: null}",
    None,
    SALT_ONE
)]
fn configured_values_reach_the_codec(
    #[case] salt: Option<&str>,
    #[case] yaml: &str,
    #[case] override_key: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(encrypt_hello(salt, yaml, override_key), expected);
}

#[test]
fn algorithm_reload_consumed() {
    let before = encrypt_hello(Some("salt-one"), "{}", None);
    let after = encrypt_hello(
        Some("salt-one"),
        "general_settings: {encryption_algorithm: aes-256-gcm}",
        None,
    );
    let reverted = encrypt_hello(
        Some("salt-one"),
        "general_settings: {encryption_algorithm: xsalsa20-poly1305}",
        None,
    );
    assert_eq!(
        [before, after, reverted],
        [SALT_ONE, SALT_ONE_AES, SALT_ONE]
    );
}
