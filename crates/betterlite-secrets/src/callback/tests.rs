#![cfg(test)]

use std::collections::BTreeSet;

use rstest::rstest;
use secrecy::SecretString;
use serde_json::{Value, json};

use super::{decrypt_callback_vars, encrypt_callback_vars, is_sensitive_callback_key};
use crate::secret_codec::diagnostics::TracingDiagnostics;
use crate::secret_codec::{Algorithm, CipherSettings, OsNonces, StoredCipher};

fn cipher(key: Option<&str>) -> StoredCipher<OsNonces, TracingDiagnostics> {
    StoredCipher {
        settings: CipherSettings {
            signing_key: key.map(SecretString::from),
            algorithm: Algorithm::Aes256Gcm,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

#[rstest]
#[case::extra_key_wins("random_field", &["random_field"], true)]
#[case::extra_key_casefold_exact("Random_Field", &["random_field"], false)]
#[case::gcs_path_any_case("GCS_Path_Service_Account", &[], true)]
#[case::masker_pattern("api_key", &[], true)]
#[case::ordinary_field("region", &[], false)]
fn reports_sensitivity(#[case] key: &str, #[case] extra: &[&str], #[case] expected: bool) {
    let extra_keys: BTreeSet<String> = extra.iter().map(ToString::to_string).collect();
    assert_eq!(is_sensitive_callback_key(key, &extra_keys), expected);
}

fn metadata() -> Value {
    json!({
        "logging": [{"callback_vars": {"api_key": "sk-secret", "region": "us-east-1"}}],
        "callback_settings": {"callback_vars": {"gcs_path_service_account": "svc.json"}},
        "other": "untouched",
    })
}

#[test]
fn encrypts_sensitive_strings_in_both_slots() {
    let extra = BTreeSet::new();
    let encrypted = encrypt_callback_vars(&metadata(), &cipher(Some("salt-one")), &extra);
    let logging = &encrypted["logging"][0]["callback_vars"];
    assert!(
        logging["api_key"]
            .as_str()
            .unwrap()
            .starts_with("litellm_enc::")
    );
    assert_eq!(logging["region"], json!("us-east-1"));
    let settings = &encrypted["callback_settings"]["callback_vars"]["gcs_path_service_account"];
    assert!(settings.as_str().unwrap().starts_with("litellm_enc::"));
    assert_eq!(encrypted["other"], json!("untouched"));
}

#[test]
fn round_trips_through_encrypt_and_decrypt() {
    let extra = BTreeSet::new();
    let codec = cipher(Some("salt-one"));
    let encrypted = encrypt_callback_vars(&metadata(), &codec, &extra);
    let decrypted = decrypt_callback_vars(&encrypted, &codec);
    assert_eq!(decrypted, metadata());
}

#[test]
fn already_marked_value_is_not_double_encrypted() {
    let extra = BTreeSet::new();
    let codec = cipher(Some("salt-one"));
    let once = encrypt_callback_vars(&metadata(), &codec, &extra);
    let twice = encrypt_callback_vars(&once, &codec, &extra);
    assert_eq!(once, twice);
}

#[test]
fn failed_encryption_keeps_plaintext() {
    let extra = BTreeSet::new();
    let encrypted = encrypt_callback_vars(&metadata(), &cipher(None), &extra);
    let logging = &encrypted["logging"][0]["callback_vars"];
    assert_eq!(logging["api_key"], json!("sk-secret"));
}

#[test]
fn failed_decryption_keeps_the_complete_marked_value() {
    let extra = BTreeSet::new();
    let marked = encrypt_callback_vars(&metadata(), &cipher(Some("salt-one")), &extra);
    let decrypted = decrypt_callback_vars(&marked, &cipher(Some("other-salt")));
    let logging = &decrypted["logging"][0]["callback_vars"];
    assert_eq!(
        logging["api_key"],
        marked["logging"][0]["callback_vars"]["api_key"]
    );
}

#[test]
fn non_dict_metadata_passes_through() {
    let codec = cipher(Some("salt-one"));
    let extra = BTreeSet::new();
    assert_eq!(
        encrypt_callback_vars(&json!("not-a-map"), &codec, &extra),
        json!("not-a-map")
    );
    assert_eq!(
        decrypt_callback_vars(&json!([1_i64, 2_i64]), &codec),
        json!([1_i64, 2_i64])
    );
}
