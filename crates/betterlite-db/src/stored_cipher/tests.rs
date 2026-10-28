#![cfg(test)]

use betterlite_secrets::error::SecretError;
use betterlite_secrets::secret_codec::{
    Algorithm, DecryptPolicy, FailureLog, FailureValue, StoredValue,
};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::new_db_cipher;

#[rstest]
#[case::legacy(Algorithm::XSalsa20Poly1305)]
#[case::aes_gcm(Algorithm::Aes256Gcm)]
fn round_trips_through_the_configured_algorithm(#[case] algorithm: Algorithm) {
    let cipher = new_db_cipher(Some(SecretString::from("salt-one")), algorithm);
    let sealed = cipher
        .encrypt_value(
            StoredValue::<()>::Text(SecretString::from("a-column-value")),
            None,
        )
        .unwrap();
    let policy = DecryptPolicy {
        log: FailureLog::Debug,
        value: FailureValue::Nothing,
    };
    let opened = cipher.decrypt_value(sealed, "column", policy);
    assert!(matches!(
        opened,
        Some(StoredValue::Text(text)) if text.expose_secret() == "a-column-value"
    ));
}

#[test]
fn a_missing_key_fails_to_encrypt() {
    let cipher = new_db_cipher(None, Algorithm::Aes256Gcm);
    let result = cipher.encrypt_value(StoredValue::<()>::Text(SecretString::from("x")), None);
    assert!(matches!(result, Err(SecretError::MissingKey)));
}

#[test]
fn a_missing_key_fails_to_decrypt_and_drops_the_value() {
    let cipher = new_db_cipher(None, Algorithm::Aes256Gcm);
    let policy = DecryptPolicy {
        log: FailureLog::Debug,
        value: FailureValue::Nothing,
    };
    let opened = cipher.decrypt_value(
        StoredValue::<()>::Text(SecretString::from("ciphertext")),
        "column",
        policy,
    );
    assert!(opened.is_none());
}

#[test]
fn a_nonstring_value_passes_through_unencrypted() {
    let cipher = new_db_cipher(Some(SecretString::from("salt-one")), Algorithm::Aes256Gcm);
    let result = cipher
        .encrypt_value(StoredValue::Other(42_i64), None)
        .unwrap();
    assert!(matches!(result, StoredValue::Other(42)));
}
