#![cfg(test)]

use std::fs;
use std::path::Path;

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretSlice, SecretString};
use serde_json::json;

use super::app_crypto::{AppCrypto, AppCryptoError, Envelope};
use crate::error::SecretError;
use crate::secret_codec::diagnostics::TracingDiagnostics;
use crate::secret_codec::{Algorithm, CipherSettings, OsNonces, StoredCipher};

// bytes(range(32)) as the raw master key, Python's cryptography.AESGCM envelope for aad b"caller-prefix"
const KEY: [u8; 32] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
    26, 27, 28, 29, 30, 31,
];
const PYTHON_NONCE: &str = "AAECAwQFBgcICQoL";
const PYTHON_CIPHERTEXT: &str = "PCC3Of/F8zetY/Wpi8laGfS5pUk=";
const PYTHON_TAG: &str = "STjChyLSESSELTqg5wuL+g==";
const AAD: &[u8] = b"caller-prefix";

fn key_secret(bytes: &[u8; 32]) -> SecretSlice<u8> {
    SecretSlice::from(bytes.to_vec())
}

#[test]
fn rejects_a_key_that_is_not_32_bytes() {
    let short = SecretSlice::from(b"too-short".to_vec());
    assert!(matches!(
        AppCrypto::new(&short),
        Err(AppCryptoError::KeyLength {
            length: 9,
            expected: 32
        })
    ));
}

#[test]
fn accepts_a_key_that_is_not_utf8() {
    let non_utf8 = SecretSlice::from(vec![0xff_u8; 32]);
    let app = AppCrypto::new(&non_utf8).unwrap();
    let data = json!({"a": 1_i64});
    let envelope = app.encrypt_json(&data, None).unwrap();
    let opened = app.decrypt_json(&envelope, None).unwrap();
    assert_eq!(opened, data);
}

#[test]
fn decrypts_a_vector_produced_by_pythons_aesgcm() {
    let app = AppCrypto::new(&key_secret(&KEY)).unwrap();
    let envelope = Envelope {
        nonce: PYTHON_NONCE.to_owned(),
        ciphertext: PYTHON_CIPHERTEXT.to_owned(),
        tag: PYTHON_TAG.to_owned(),
    };
    let data = app.decrypt_json(&envelope, Some(AAD)).unwrap();
    assert_eq!(data, json!({"a": 1_i64, "b": "two"}));
}

#[test]
fn round_trips_and_a_python_reader_would_accept_the_shape() {
    let app = AppCrypto::new(&key_secret(&KEY)).unwrap();
    let data = json!({"a": 1_i64, "b": "two"});
    let envelope = app.encrypt_json(&data, Some(AAD)).unwrap();
    let opened = app.decrypt_json(&envelope, Some(AAD)).unwrap();
    assert_eq!(opened, data);
}

#[test]
fn wrong_aad_fails_authentication() {
    let app = AppCrypto::new(&key_secret(&KEY)).unwrap();
    let envelope = Envelope {
        nonce: PYTHON_NONCE.to_owned(),
        ciphertext: PYTHON_CIPHERTEXT.to_owned(),
        tag: PYTHON_TAG.to_owned(),
    };
    let error = app.decrypt_json(&envelope, None).unwrap_err();
    assert!(matches!(error, AppCryptoError::Seal { .. }));
    // cryptography's InvalidTag carries no text of its own; str(error) is "".
    assert_eq!(error.to_string(), "");
}

#[rstest]
#[case::tampered_ciphertext(PYTHON_CIPHERTEXT.replacen('C', "D", 1), PYTHON_TAG.to_owned())]
#[case::tampered_tag(PYTHON_CIPHERTEXT.to_owned(), PYTHON_TAG.replacen('S', "T", 1))]
fn tampering_fails_authentication(#[case] ciphertext: String, #[case] tag: String) {
    let app = AppCrypto::new(&key_secret(&KEY)).unwrap();
    let envelope = Envelope {
        nonce: PYTHON_NONCE.to_owned(),
        ciphertext,
        tag,
    };
    assert!(matches!(
        app.decrypt_json(&envelope, Some(AAD)),
        Err(AppCryptoError::Seal { .. })
    ));
}

fn fuzz_cipher() -> StoredCipher<OsNonces, TracingDiagnostics> {
    StoredCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from("salt-one")),
            algorithm: Algorithm::Aes256Gcm,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

#[test]
fn decode_stored_secret_accepts_a_known_aes_value() {
    let cipher = fuzz_cipher();
    let text = "v2:gcm:AAECAwQFBgcICQoLu13Sfnjzb3vLfCPO7ClYdksHnW1L";
    let opened = cipher.decode_stored_secret(text.as_bytes());
    assert_eq!(
        opened.map(|value| value.expose_secret().to_owned()),
        Some("hello".to_owned())
    );
}

#[test]
fn decode_stored_secret_rejects_invalid_utf8() {
    let cipher = fuzz_cipher();
    assert!(cipher.decode_stored_secret(&[0xff, 0xfe]).is_none());
}

#[test]
fn decode_secret_map_accepts_a_plaintext_map() {
    let cipher = fuzz_cipher();
    let map = cipher.decode_secret_map(br#"{"a":"1"}"#).unwrap().unwrap();
    assert_eq!(
        map.get("a").map(|value| value.expose_secret().to_owned()),
        Some("1".to_owned())
    );
}

#[test]
fn decode_secret_map_rejects_a_wrong_field_type() {
    let cipher = fuzz_cipher();
    assert!(matches!(
        cipher.decode_secret_map(br#"{"a":1}"#),
        Err(SecretError::MapDecode { .. })
    ));
}

#[test]
fn replays_the_stored_secret_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/stored_secret");
    let cipher = fuzz_cipher();
    let seeds: Vec<Vec<u8>> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| fs::read(entry.unwrap().path()).unwrap())
        .collect();
    let opened = seeds.iter().filter(|seed| {
        let Some((&tag, bytes)) = seed.split_first() else {
            return false;
        };
        if tag.is_multiple_of(2) {
            cipher.decode_stored_secret(bytes).is_some()
        } else {
            cipher
                .decode_secret_map(bytes)
                .is_ok_and(|decoded| decoded.is_some())
        }
    });
    assert_eq!((seeds.len(), opened.count()), (5, 3));
}
