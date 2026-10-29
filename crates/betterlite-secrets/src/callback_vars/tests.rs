#![cfg(test)]

use std::collections::BTreeSet;
use std::sync::Mutex;

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

use super::{Transform, is_sensitive_callback_key, strip_callback_config, transform_callback_vars};
use crate::secret_codec::diagnostics::{Level, SecretDiagnostics, SecretEvent, TracingDiagnostics};
use crate::secret_codec::{
    Algorithm, CALLBACK_VAR_ENCRYPTED_PREFIX, CipherSettings, NonceSource, OsNonces, StoredCipher,
    StoredValue,
};

fn cipher_with<D>(key: Option<&str>, diagnostics: D) -> StoredCipher<OsNonces, D> {
    StoredCipher {
        settings: CipherSettings {
            signing_key: key.map(SecretString::from),
            algorithm: Algorithm::Aes256Gcm,
        },
        nonces: OsNonces,
        diagnostics,
    }
}

fn cipher(key: Option<&str>) -> StoredCipher<OsNonces, TracingDiagnostics> {
    cipher_with(key, TracingDiagnostics)
}

fn encrypt_callback_vars<N, D>(metadata: &Value, cipher: &StoredCipher<N, D>) -> Value
where
    N: NonceSource,
    D: SecretDiagnostics,
{
    transform_callback_vars(metadata, &Transform::Encrypt(cipher))
}

fn decrypt_callback_vars<N, D>(metadata: &Value, cipher: &StoredCipher<N, D>) -> Value
where
    N: NonceSource,
    D: SecretDiagnostics,
{
    transform_callback_vars(metadata, &Transform::Decrypt(cipher))
}

fn sample_metadata() -> Value {
    json!({
        "logging": [
            {"callback_vars": {"api_key": "sk-secret", "region": "us-east-1"}},
            "not_a_dict",
        ],
        "callback_settings": {"callback_vars": {"gcs_path_service_account": "svc.json"}},
        "secret_manager_settings": {"aws_access_key_id": "AKIA-secret"},
        "other": "untouched",
    })
}

#[test]
fn non_dict_unchanged() {
    assert_eq!(
        strip_callback_config(&json!("not-a-map")),
        json!("not-a-map")
    );
    assert_eq!(strip_callback_config(&Value::Null), Value::Null);
}

#[test]
fn strip_three_top_level_slots() {
    let stripped = strip_callback_config(&sample_metadata());
    assert_eq!(stripped, json!({"other": "untouched"}));
}

#[test]
fn strip_nested_names_preserved() {
    let metadata = json!({
        "other": {"logging": "nested-untouched", "callback_settings": "also-untouched"},
        "logging": [],
    });
    let stripped = strip_callback_config(&metadata);
    assert_eq!(
        stripped,
        json!({"other": {"logging": "nested-untouched", "callback_settings": "also-untouched"}})
    );
}

#[test]
fn deep_copy_no_mutation() {
    let original = sample_metadata();
    let snapshot = original.clone();
    let codec = cipher(Some("salt-one"));
    drop(encrypt_callback_vars(&original, &codec));
    drop(decrypt_callback_vars(&original, &codec));
    drop(strip_callback_config(&original));
    assert_eq!(original, snapshot);
}

#[test]
fn logging_and_callback_settings_slots() {
    let codec = cipher(Some("salt-one"));
    let encrypted = encrypt_callback_vars(&sample_metadata(), &codec);
    let logging_vars = &encrypted["logging"][0]["callback_vars"]["api_key"];
    assert!(
        logging_vars
            .as_str()
            .unwrap()
            .starts_with(CALLBACK_VAR_ENCRYPTED_PREFIX)
    );
    let settings_vars =
        &encrypted["callback_settings"]["callback_vars"]["gcs_path_service_account"];
    assert!(
        settings_vars
            .as_str()
            .unwrap()
            .starts_with(CALLBACK_VAR_ENCRYPTED_PREFIX)
    );
}

#[test]
fn malformed_slots_ignored() {
    let codec = cipher(Some("salt-one"));
    let scalar_logging =
        json!({"logging": "not-a-list", "callback_settings": {"callback_vars": "not-a-dict"}});
    assert_eq!(
        encrypt_callback_vars(&scalar_logging, &codec),
        scalar_logging
    );
    let mixed_entries =
        json!({"logging": [{"callback_vars": {"api_key": "sk-secret"}}, "not_a_dict", 7_i32]});
    let encrypted = encrypt_callback_vars(&mixed_entries, &codec);
    assert_eq!(encrypted["logging"][1], json!("not_a_dict"));
    assert_eq!(encrypted["logging"][2], json!(7_i32));
}

#[test]
fn nested_values_not_recursed() {
    let metadata = json!({"callback_settings": {"callback_vars": {"api_key": {"nested_api_key": "sk-secret"}}}});
    let encrypted = encrypt_callback_vars(&metadata, &cipher(Some("salt-one")));
    assert_eq!(encrypted, metadata);
}

#[test]
fn secret_manager_not_encrypted() {
    let metadata = json!({"secret_manager_settings": {"aws_access_key_id": "AKIA-secret"}});
    let encrypted = encrypt_callback_vars(&metadata, &cipher(Some("salt-one")));
    assert_eq!(encrypted, metadata);
}

#[rstest]
#[case::credential_segments("access-token", &[][..], true)]
#[case::gcs_key_casefold("GCS_Path_Service_Account", &[][..], true)]
#[case::cost_exemption("input_cost_per_token", &[][..], false)]
#[case::plural_credentials("vertex_credentials", &[][..], true)]
#[case::extra_exact_match("input_cost_per_token", &["input_cost_per_token"][..], true)]
fn sensitive_key_selection(#[case] key: &str, #[case] extra: &[&str], #[case] expected: bool) {
    let extra_keys: BTreeSet<String> = extra.iter().map(ToString::to_string).collect();
    assert_eq!(is_sensitive_callback_key(key, &extra_keys), expected);
}

#[test]
fn empty_and_non_string() {
    let codec = cipher(Some("salt-one"));
    let empty = json!({"callback_settings": {"callback_vars": {"api_key": ""}}});
    assert_eq!(encrypt_callback_vars(&empty, &codec), empty);
    let number = json!({"callback_settings": {"callback_vars": {"api_key": 7_i32}}});
    assert_eq!(encrypt_callback_vars(&number, &codec), number);
}

#[test]
fn nonsensitive_unchanged() {
    let metadata = json!({"callback_settings": {"callback_vars": {"region": "us-east-1"}}});
    let encrypted = encrypt_callback_vars(&metadata, &cipher(Some("salt-one")));
    assert_eq!(encrypted, metadata);
}

#[test]
fn prefix_without_validation() {
    let marked = json!("litellm_enc::not-even-base64*garbage");
    let metadata = json!({"callback_settings": {"callback_vars": {"api_key": marked}}});
    let encrypted = encrypt_callback_vars(&metadata, &cipher(Some("salt-one")));
    assert_eq!(encrypted, metadata);
}

#[test]
fn salt_rotation_idempotence() {
    let metadata = json!({"callback_settings": {"callback_vars": {"api_key": "sk-secret"}}});
    let once = encrypt_callback_vars(&metadata, &cipher(Some("salt-one")));
    let twice = encrypt_callback_vars(&once, &cipher(Some("salt-two")));
    assert_eq!(once, twice);
}

#[test]
fn successful_encrypt_prefix() {
    let metadata = json!({"callback_settings": {"callback_vars": {"api_key": "sk-secret"}}});
    let encrypted = encrypt_callback_vars(&metadata, &cipher(Some("salt-one")));
    let value = encrypted["callback_settings"]["callback_vars"]["api_key"]
        .as_str()
        .unwrap();
    assert!(value.starts_with(CALLBACK_VAR_ENCRYPTED_PREFIX));
    assert_ne!(value, "sk-secret");
}

#[test]
fn encryption_error_plaintext_silent() {
    let metadata = json!({"callback_settings": {"callback_vars": {"api_key": "sk-secret"}}});
    let encrypted = encrypt_callback_vars(&metadata, &cipher(None));
    assert_eq!(encrypted, metadata);
}

#[test]
fn legacy_plaintext_decrypt() {
    let metadata = json!({"callback_settings": {"callback_vars": {"api_key": "plain-value"}}});
    let decrypted = decrypt_callback_vars(&metadata, &cipher(Some("salt-one")));
    assert_eq!(decrypted, metadata);
}

#[test]
fn decrypt_ignores_key_sensitivity() {
    let codec = cipher(Some("salt-one"));
    let encrypted = encrypt_callback_vars(
        &json!({"callback_settings": {"callback_vars": {"api_key": "sk-secret"}}}),
        &codec,
    );
    let marked = encrypted["callback_settings"]["callback_vars"]["api_key"].clone();
    let metadata = json!({"callback_settings": {"callback_vars": {"region": marked}}});
    let decrypted = decrypt_callback_vars(&metadata, &codec);
    assert_eq!(
        decrypted["callback_settings"]["callback_vars"]["region"],
        json!("sk-secret")
    );
}

#[test]
fn decrypted_empty_string() {
    let codec = cipher(Some("salt-one"));
    let sealed = codec
        .encrypt_value(StoredValue::<()>::Text(SecretString::from("")), None)
        .unwrap();
    let StoredValue::Text(sealed_text) = sealed else {
        panic!("expected a sealed text value")
    };
    let marked = format!(
        "{CALLBACK_VAR_ENCRYPTED_PREFIX}{}",
        sealed_text.expose_secret()
    );
    let metadata = json!({"callback_settings": {"callback_vars": {"api_key": marked}}});
    let decrypted = decrypt_callback_vars(&metadata, &codec);
    assert_eq!(
        decrypted["callback_settings"]["callback_vars"]["api_key"],
        json!("")
    );
}

#[test]
fn corrupt_ciphertext_retains_prefix() {
    let codec = cipher(Some("salt-one"));
    let marked = format!("{CALLBACK_VAR_ENCRYPTED_PREFIX}not-valid-ciphertext");
    let metadata = json!({"callback_settings": {"callback_vars": {"api_key": marked}}});
    let decrypted = decrypt_callback_vars(&metadata, &codec);
    assert_eq!(
        decrypted["callback_settings"]["callback_vars"]["api_key"],
        metadata["callback_settings"]["callback_vars"]["api_key"]
    );
}

struct CapturingDiagnostics {
    messages: Mutex<Vec<String>>,
}

impl SecretDiagnostics for CapturingDiagnostics {
    fn record(&self, _level: Level, event: &SecretEvent<'_>) {
        if let Ok(mut messages) = self.messages.lock() {
            messages.push(event.message());
        }
    }
}

fn capturing_cipher(key: Option<&str>) -> StoredCipher<OsNonces, CapturingDiagnostics> {
    cipher_with(
        key,
        CapturingDiagnostics {
            messages: Mutex::new(Vec::new()),
        },
    )
}

fn assert_no_leak(
    codec: &StoredCipher<OsNonces, CapturingDiagnostics>,
    plaintext: &str,
    ciphertext: &str,
) {
    let recorded = codec.diagnostics.messages.lock().unwrap();
    let all_redacted = recorded
        .iter()
        .all(|message| !message.contains(plaintext) && !message.contains(ciphertext));
    drop(recorded);
    assert!(all_redacted);
}

#[test]
fn secret_debug_redaction() {
    let plaintext = "sk-super-secret-value";
    let metadata = json!({"callback_settings": {"callback_vars": {"api_key": plaintext}}});

    let encrypt_codec = capturing_cipher(Some("salt-one"));
    let encrypted = encrypt_callback_vars(&metadata, &encrypt_codec);
    let ciphertext = encrypted["callback_settings"]["callback_vars"]["api_key"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_ne!(ciphertext, plaintext);
    assert_no_leak(&encrypt_codec, plaintext, &ciphertext);

    let missing_key_codec = capturing_cipher(None);
    let fallback = encrypt_callback_vars(&metadata, &missing_key_codec);
    assert_eq!(fallback, metadata);
    assert_no_leak(&missing_key_codec, plaintext, &ciphertext);

    let decrypt_codec = capturing_cipher(Some("salt-one"));
    let decrypted = decrypt_callback_vars(&encrypted, &decrypt_codec);
    assert_eq!(
        decrypted["callback_settings"]["callback_vars"]["api_key"],
        json!(plaintext)
    );
    assert_no_leak(&decrypt_codec, plaintext, &ciphertext);

    let wrong_salt_codec = capturing_cipher(Some("salt-two"));
    let unchanged = decrypt_callback_vars(&encrypted, &wrong_salt_codec);
    assert_eq!(unchanged, encrypted);
    let recorded = wrong_salt_codec.diagnostics.messages.lock().unwrap();
    let is_empty = recorded.is_empty();
    drop(recorded);
    assert!(!is_empty);
    assert_no_leak(&wrong_salt_codec, plaintext, &ciphertext);
}
