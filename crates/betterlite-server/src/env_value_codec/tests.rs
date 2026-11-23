#![cfg(test)]

use std::sync::Mutex;

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::Object as JsonMap;
use betterlite_secrets::error::SecretError;
use betterlite_secrets::secret_codec::diagnostics::{Level, SecretDiagnostics, SecretEvent};
use betterlite_secrets::secret_codec::{
    Algorithm, CipherSettings, NonceSource, StoredCipher, decrypt_if_encrypted_with,
};

use super::{EnvValueCodecError, decrypt_env, encrypt_env, encrypt_env_for_db};

// LiteLLM's own vectors, salt "salt-one" and nonce bytes 0, 1, 2, ...
const SALT_KEY: &str = "salt-one";
const OVERRIDE_KEY: &str = "override-key";
const LEGACY_HELLO: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXccVZjgu1TZy2W9RyeYfnLiSlXcAj";
const AES_HELLO: &str = "v2:gcm:AAECAwQFBgcICQoLu13Sfnjzb3vLfCPO7ClYdksHnW1L";
const LEGACY: Algorithm = Algorithm::XSalsa20Poly1305;
const AES: Algorithm = Algorithm::Aes256Gcm;

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

#[derive(Default)]
struct Recorder(Mutex<Vec<(Level, String)>>);

impl SecretDiagnostics for Recorder {
    fn record(&self, level: Level, event: &SecretEvent<'_>) {
        self.0.lock().unwrap().push((level, event.message()));
    }
}

type Cipher = StoredCipher<CountingNonces, Recorder>;

fn test_cipher(key: Option<&str>, algorithm: Algorithm) -> Cipher {
    StoredCipher {
        settings: CipherSettings {
            signing_key: key.map(SecretString::from),
            algorithm,
        },
        nonces: CountingNonces,
        diagnostics: Recorder::default(),
    }
}

fn object(entries: &[(&str, JsonValue)]) -> JsonMap {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

fn nested(entries: &[(&str, JsonValue)]) -> JsonValue {
    JsonValue::Object(object(entries))
}

fn opened(value: &str, key: &str) -> Option<String> {
    decrypt_if_encrypted_with(value, &SecretString::from(key))
        .map(|text| text.expose_secret().to_owned())
}

fn messages(cipher: &Cipher) -> Vec<(Level, String)> {
    cipher.diagnostics.0.lock().unwrap().clone()
}

#[rstest]
#[case::legacy(LEGACY)]
#[case::aes(AES)]
fn encrypt_env_seals_every_string_and_passes_other_values(#[case] algorithm: Algorithm) {
    let cipher = test_cipher(Some(SALT_KEY), algorithm);
    let values = object(&[
        ("api_key", JsonValue::from("sk-secret")),
        ("model", JsonValue::from("gpt-4o")),
        ("count", JsonValue::from(3_u32)),
        ("enabled", JsonValue::Bool(true)),
        ("nothing", JsonValue::Null),
        (
            "names",
            JsonValue::Array(vec![JsonValue::from("a"), JsonValue::from("b")]),
        ),
        ("nested", nested(&[("port", JsonValue::from(6379_u32))])),
    ]);
    let encrypted = encrypt_env(&values, &cipher, None).unwrap();
    assert_eq!(encrypted.len(), values.len());
    assert_eq!(encrypted["count"], values["count"]);
    assert_eq!(encrypted["enabled"], values["enabled"]);
    assert_eq!(encrypted["nothing"], values["nothing"]);
    assert_eq!(encrypted["names"], values["names"]);
    assert_eq!(encrypted["nested"], values["nested"]);
    let sealed_key = encrypted["api_key"].as_str().unwrap();
    assert_ne!(sealed_key, "sk-secret");
    assert_eq!(opened(sealed_key, SALT_KEY).as_deref(), Some("sk-secret"));
    let sealed_model = encrypted["model"].as_str().unwrap();
    assert_eq!(opened(sealed_model, SALT_KEY).as_deref(), Some("gpt-4o"));
    assert_eq!(sealed_key.starts_with("v2:gcm:"), algorithm == AES);
    assert_eq!(decrypt_env(&encrypted, &cipher), values);
    let recorded = messages(&cipher);
    assert_eq!(recorded.len(), 5);
    assert!(recorded.iter().all(|(level, _)| *level == Level::Debug));
}

#[rstest]
#[case::legacy(LEGACY)]
#[case::aes(AES)]
fn encrypt_env_uses_a_non_empty_new_key_and_falls_back_from_an_empty_one(
    #[case] algorithm: Algorithm,
) {
    let cipher = test_cipher(Some(SALT_KEY), algorithm);
    let values = object(&[("api_key", JsonValue::from("sk-secret"))]);
    let override_key = SecretString::from(OVERRIDE_KEY);
    let sealed = encrypt_env(&values, &cipher, Some(&override_key)).unwrap();
    let under_override = sealed["api_key"].as_str().unwrap();
    assert_eq!(
        opened(under_override, OVERRIDE_KEY).as_deref(),
        Some("sk-secret")
    );
    assert_eq!(opened(under_override, SALT_KEY), None);

    let empty_key = SecretString::from("");
    let fell_back = encrypt_env(&values, &cipher, Some(&empty_key)).unwrap();
    let under_salt = fell_back["api_key"].as_str().unwrap();
    assert_eq!(opened(under_salt, SALT_KEY).as_deref(), Some("sk-secret"));
    assert_eq!(opened(under_salt, OVERRIDE_KEY), None);
}

#[test]
fn a_string_without_a_configured_key_fails_naming_the_key_only() {
    let cipher = test_cipher(None, LEGACY);
    let values = object(&[("api_key", JsonValue::from("sk-live-secret"))]);
    let error = encrypt_env(&values, &cipher, None).unwrap_err();
    let EnvValueCodecError::Encrypt { key, source } = &error;
    assert_eq!(key, "api_key");
    assert!(matches!(source, SecretError::MissingKey));
    let text = error.to_string();
    assert!(text.contains("api_key"), "{text}");
    assert!(!text.contains("sk-live"), "{text}");
    assert!(messages(&cipher).is_empty());
}

#[test]
fn a_map_of_non_strings_needs_no_configured_key() {
    let cipher = test_cipher(None, LEGACY);
    let values = object(&[
        ("port", JsonValue::from(6379_u32)),
        ("ssl", JsonValue::Bool(true)),
        ("nothing", JsonValue::Null),
        (
            "nodes",
            JsonValue::Array(vec![JsonValue::Array(vec![
                JsonValue::from("127.0.0.1"),
                JsonValue::from(7000_u32),
            ])]),
        ),
        ("nested", nested(&[("password", JsonValue::from("plain"))])),
    ]);
    assert_eq!(encrypt_env(&values, &cipher, None).unwrap(), values);
    assert_eq!(decrypt_env(&values, &cipher), values);
    let recorded = messages(&cipher);
    assert_eq!(recorded.len(), 5);
    assert!(recorded.iter().all(
        |(level, message)| *level == Level::Debug && message.contains("value must be a string")
    ));
}

#[rstest]
#[case::legacy(LEGACY)]
#[case::aes(AES)]
fn decrypt_env_opens_what_it_can(#[case] algorithm: Algorithm) {
    let cipher = test_cipher(Some(SALT_KEY), algorithm);
    let readable = object(&[
        ("legacy", JsonValue::from(LEGACY_HELLO)),
        ("sealed", JsonValue::from(AES_HELLO)),
    ]);
    let decrypted = decrypt_env(&readable, &cipher);
    assert_eq!(decrypted["legacy"], "hello");
    assert_eq!(decrypted["sealed"], "hello");
    assert!(messages(&cipher).is_empty());
}

#[rstest]
#[case::legacy(LEGACY)]
#[case::aes(AES)]
fn decrypt_env_keeps_the_rest_as_stored(#[case] algorithm: Algorithm) {
    let cipher = test_cipher(Some(SALT_KEY), algorithm);
    let foreign = test_cipher(Some("salt-other"), algorithm);
    let foreign_sealed = encrypt_env(
        &object(&[("host", JsonValue::from("foreign-host"))]),
        &foreign,
        None,
    )
    .unwrap();
    let values = object(&[
        ("host", foreign_sealed["host"].clone()),
        ("garbage", JsonValue::from("not*base64*text")),
        ("plain", JsonValue::from("plaintext")),
        ("port", JsonValue::from(6379_u32)),
    ]);
    let kept = decrypt_env(&values, &cipher);
    assert_eq!(kept, values);
    let recorded = messages(&cipher);
    assert_eq!(recorded.len(), 3);
    assert!(recorded.iter().all(|(level, _)| *level == Level::Debug));
    let mut texts: Vec<String> = recorded
        .iter()
        .map(|(_, message)| message.clone())
        .collect();
    texts.sort();
    let mut expected = vec![
        "Unable to decrypt value for key: host, returning None".to_owned(),
        "Unable to decrypt value for key: garbage, returning None".to_owned(),
        "Unable to decrypt value for key: plain, returning None".to_owned(),
    ];
    expected.sort();
    assert_eq!(texts, expected);
    assert!(
        texts
            .iter()
            .all(|text| !text.contains("foreign-host") && !text.contains("plaintext"))
    );
}

#[rstest]
#[case::legacy(LEGACY)]
#[case::aes(AES)]
fn encrypt_env_for_db_opens_plaintext_and_stored_ciphertext_once(#[case] algorithm: Algorithm) {
    let cipher = test_cipher(Some(SALT_KEY), algorithm);
    let values = object(&[
        ("plain", JsonValue::from("hello")),
        ("legacy", JsonValue::from(LEGACY_HELLO)),
        ("sealed", JsonValue::from(AES_HELLO)),
        ("port", JsonValue::from(6379_u32)),
    ]);
    let stored = encrypt_env_for_db(&values, &cipher, None).unwrap();
    assert_eq!(stored.len(), values.len());
    assert_eq!(stored["port"], values["port"]);
    let plain = stored["plain"].as_str().unwrap();
    assert_eq!(opened(plain, SALT_KEY).as_deref(), Some("hello"));
    assert_eq!(plain.starts_with("v2:gcm:"), algorithm == AES);
    let legacy = stored["legacy"].as_str().unwrap();
    assert_eq!(opened(legacy, SALT_KEY).as_deref(), Some("hello"));
    let sealed = stored["sealed"].as_str().unwrap();
    assert_eq!(opened(sealed, SALT_KEY).as_deref(), Some("hello"));
}

#[rstest]
#[case::legacy(LEGACY)]
#[case::aes(AES)]
fn encrypt_env_for_db_decrypts_with_the_configured_key_before_an_override_write(
    #[case] algorithm: Algorithm,
) {
    let cipher = test_cipher(Some(SALT_KEY), algorithm);
    let values = object(&[
        ("legacy", JsonValue::from(LEGACY_HELLO)),
        ("sealed", JsonValue::from(AES_HELLO)),
        ("plain", JsonValue::from("hello")),
    ]);
    let override_key = SecretString::from(OVERRIDE_KEY);
    let stored = encrypt_env_for_db(&values, &cipher, Some(&override_key)).unwrap();
    let legacy = stored["legacy"].as_str().unwrap();
    let sealed = stored["sealed"].as_str().unwrap();
    let plain = stored["plain"].as_str().unwrap();
    assert_eq!(opened(legacy, OVERRIDE_KEY).as_deref(), Some("hello"));
    assert_eq!(opened(sealed, OVERRIDE_KEY).as_deref(), Some("hello"));
    assert_eq!(opened(plain, OVERRIDE_KEY).as_deref(), Some("hello"));
    assert_eq!(opened(legacy, SALT_KEY), None);
}

#[rstest]
#[case::legacy(LEGACY)]
#[case::aes(AES)]
fn encrypt_env_for_db_turns_star_and_empty_into_sealed_empty_text(#[case] algorithm: Algorithm) {
    let cipher = test_cipher(Some(SALT_KEY), algorithm);
    let values = object(&[
        ("star", JsonValue::from("*")),
        ("empty", JsonValue::from("")),
    ]);
    let encrypted = encrypt_env(&values, &cipher, None).unwrap();
    assert_eq!(
        opened(encrypted["star"].as_str().unwrap(), SALT_KEY).as_deref(),
        Some("*")
    );
    assert_eq!(
        opened(encrypted["empty"].as_str().unwrap(), SALT_KEY).as_deref(),
        Some("")
    );
    let stored = encrypt_env_for_db(&values, &cipher, None).unwrap();
    let star = stored["star"].as_str().unwrap();
    let empty = stored["empty"].as_str().unwrap();
    assert_ne!(star, "*");
    assert_eq!(opened(star, SALT_KEY).as_deref(), Some(""));
    assert_eq!(opened(empty, SALT_KEY).as_deref(), Some(""));
}
