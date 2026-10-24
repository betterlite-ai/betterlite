#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::str;
use std::sync::Mutex;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rstest::rstest;
use secrecy::SecretString;

use crate::error::SecretError;
use crate::secret_codec::bearer::{decrypt_bearer_token, encrypt_bearer_token};
use crate::secret_codec::diagnostics::{Level, SecretDiagnostics, SecretEvent};
use crate::secret_codec::sealing::seal;
use crate::secret_codec::secret_map::{StoredMap, decode_secret_map, encrypt_secret_map};
use crate::secret_codec::{
    Algorithm, CipherSettings, DecryptPolicy, FailureLog, FailureValue, MAX_ENCODED_BYTES,
    NonceSource, StoredCipher, StoredValue, decrypt_if_encrypted_with,
};

// every vector below came from LiteLLM's own helpers, salt "salt-one", nonce bytes 0, 1, 2, ...
const LEGACY_HELLO: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXccVZjgu1TZy2W9RyeYfnLiSlXcAj";
const AES_HELLO: &str = "v2:gcm:AAECAwQFBgcICQoLu13Sfnjzb3vLfCPO7ClYdksHnW1L";
const LONG_STANDARD: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXEHhsXXGE41XMBOZQ9MNoYCQDmMAgx/StIWW/bAyTWAHjflBfc16NcjLyZVRCThkL9tn/hno=";
const OVERRIDE_HELLO: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXfuRp9WUvbNm7fn_PA-d4hvaoT58a";
const BEARER: &str = "litellm_login_AAECAwQFBgcICQoLo1nHfnhTvgR3dDrQSldam-gVqoGVzw8";
const PREFIX: &str = "litellm_login_";
const MAP_AES: &str = r#""v2:gcm:AAECAwQFBgcICQoLqBrfMC0Q6wYQjNwMNTf7eipqg70QyiuAwg==""#;
const MAP_PYTHON_ORDER: &str =
    r#""AAECAwQFBgcICQoLDA0ODxAREhMUFRYXyOd3Se2GVAYxQwAxhBdAdjfiU452iub4zvGsIlKdSUK-""#;
const EMPTY_MAP_CIPHER: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXPg3fa5gBvN9CDmncHUEtnze9";
const SALT: Option<&str> = Some("salt-one");
const LEGACY: Algorithm = Algorithm::XSalsa20Poly1305;
const AES: Algorithm = Algorithm::Aes256Gcm;
const REJECTED: &str = "error: Cannot decode encrypted MCP headers; check LITELLM_SALT_KEY";

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

fn build(key: Option<&str>, algorithm: Algorithm) -> Cipher {
    StoredCipher {
        settings: CipherSettings {
            signing_key: key.map(SecretString::from),
            algorithm,
        },
        nonces: CountingNonces,
        diagnostics: Recorder::default(),
    }
}

fn logs(cipher: &Cipher) -> Vec<(Level, String)> {
    cipher.diagnostics.0.lock().unwrap().clone()
}

fn encrypt(cipher: &Cipher, value: &str, override_key: Option<&str>) -> Option<String> {
    let key = override_key.map(SecretString::from);
    match cipher.encrypt_value(StoredValue::<()>::Text(value.to_owned()), key.as_ref()) {
        Ok(StoredValue::Text(text)) => Some(text),
        Ok(StoredValue::Other(())) => None,
        Err(error) => Some(format!("error: {error}")),
    }
}

fn decrypt(
    cipher: &Cipher,
    value: &str,
    log: FailureLog,
    fallback: FailureValue,
) -> Option<String> {
    let policy = DecryptPolicy {
        log,
        value: fallback,
    };
    match cipher.decrypt_value(StoredValue::<()>::Text(value.to_owned()), "api_key", policy) {
        Some(StoredValue::Text(text)) => Some(text),
        Some(StoredValue::Other(())) | None => None,
    }
}

fn loud(cipher: &Cipher, value: &str) -> Option<String> {
    decrypt(cipher, value, FailureLog::Error, FailureValue::Nothing)
}

fn probe_with(value: &str, key: &str) -> Option<String> {
    decrypt_if_encrypted_with(value, &SecretString::from(key))
}

fn decode(cipher: &Cipher, stored: Option<&StoredMap>) -> String {
    match decode_secret_map(cipher, stored, "headers") {
        Ok(map) => serde_json::to_string(&map).unwrap(),
        Err(error) => format!("error: {error}"),
    }
}

fn decode_text(cipher: &Cipher, value: &str) -> String {
    decode(cipher, Some(&StoredMap::Text(value.to_owned())))
}

fn utf8_bearer(cipher: &Cipher) -> Option<String> {
    let key = SecretString::from("salt-one");
    let sealed = seal(AES, &key, &CountingNonces, &[0xff], PREFIX.as_bytes()).unwrap();
    decrypt_bearer_token(
        cipher,
        &format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(sealed)),
        PREFIX,
    )
}

fn bad_bearers(cipher: &Cipher) -> Option<String> {
    let wrong_prefix = decrypt_bearer_token(cipher, BEARER, "other_");
    let alphabet = decrypt_bearer_token(cipher, &BEARER.replace('-', "*"), PREFIX);
    let length = decrypt_bearer_token(cipher, &format!("{BEARER}AA"), PREFIX);
    let tag = decrypt_bearer_token(cipher, &BEARER.replace('Q', "R"), PREFIX);
    [wrong_prefix, alphabet, length, tag, utf8_bearer(cipher)]
        .into_iter()
        .flatten()
        .next()
}

fn passthrough(cipher: &Cipher) -> String {
    let encrypted = cipher.encrypt_value(StoredValue::Other(7_u8), None).ok();
    let policy = DecryptPolicy {
        log: FailureLog::Error,
        value: FailureValue::Nothing,
    };
    let decrypted = cipher.decrypt_value(StoredValue::Other(8_u8), "k", policy);
    format!("{encrypted:?} {decrypted:?}")
}

#[rstest]
#[case::legacy_nonce_mac_layout(SALT, LEGACY, |cipher: &Cipher| encrypt(cipher, "hello", None), Some(LEGACY_HELLO), &[])]
#[case::aes_prefix_nonce_tag_layout(SALT, AES, |cipher: &Cipher| encrypt(cipher, "hello", None), Some(AES_HELLO), &[])]
#[case::override_key_writes(SALT, LEGACY, |cipher: &Cipher| encrypt(cipher, "hello", Some("override-key")), Some(OVERRIDE_HELLO), &[])]
#[case::empty_override_falls_back(SALT, LEGACY, |cipher: &Cipher| encrypt(cipher, "hello", Some("")), Some(LEGACY_HELLO), &[])]
#[case::override_reads_back(Some("override-key"), LEGACY, |cipher: &Cipher| loud(cipher, OVERRIDE_HELLO), Some("hello"), &[])]
#[case::legacy_standard_base64(SALT, LEGACY, |cipher: &Cipher| loud(cipher, LONG_STANDARD), Some("h\u{e9}llo w\u{f6}rld, a longer secret value!"), &[])]
#[case::permissive_legacy_decode(SALT, LEGACY, |cipher: &Cipher| loud(cipher, "AAEC*AwQFBgc\nICQoLDA0ODxAREhMUFRYXccVZjgu1TZy2W9RyeYfnLiSlXcAj"), Some("hello"), &[])]
#[case::permissive_standard_fallback(SALT, LEGACY, |cipher: &Cipher| loud(cipher, &format!("{LEGACY_HELLO}-")), Some("hello"), &[])]
#[case::permissive_stray_pad_rejected(SALT, LEGACY, |cipher: &Cipher| loud(cipher, &format!("{LEGACY_HELLO}=x=")), None, &[Level::Debug, Level::Error])]
#[case::empty_legacy_bytes(SALT, LEGACY, |cipher: &Cipher| loud(cipher, "*"), Some(""), &[])]
#[case::empty_legacy_bytes_need_a_key(None, LEGACY, |cipher: &Cipher| loud(cipher, ""), None, &[Level::Debug, Level::Error])]
#[case::algorithm_switch_reads_old(SALT, AES, |cipher: &Cipher| loud(cipher, LEGACY_HELLO), Some("hello"), &[])]
#[case::algorithm_switch_reads_new(SALT, LEGACY, |cipher: &Cipher| loud(cipher, AES_HELLO), Some("hello"), &[])]
#[case::string_encrypt_failure(None, AES, |cipher: &Cipher| encrypt(cipher, "hello", None), Some("error: no LITELLM_SALT_KEY or master key is set to encrypt with"), &[])]
#[case::non_string_passthrough(None, LEGACY, |cipher: &Cipher| Some(passthrough(cipher)), Some("Some(Other(7)) Some(Other(8))"), &[Level::Debug])]
#[case::decrypt_debug_policy(SALT, LEGACY, |cipher: &Cipher| decrypt(cipher, "v2:gcm:AAEC", FailureLog::Debug, FailureValue::Original), Some("v2:gcm:AAEC"), &[Level::Debug])]
#[case::decrypt_debug_policy_none(SALT, LEGACY, |cipher: &Cipher| decrypt(cipher, "v2:gcm:AAEC", FailureLog::Debug, FailureValue::Nothing), None, &[Level::Debug])]
#[case::decrypt_error_policy(SALT, LEGACY, |cipher: &Cipher| loud(cipher, "AAAA-"), None, &[Level::Debug, Level::Error])]
#[case::decrypt_error_policy_original(SALT, LEGACY, |cipher: &Cipher| decrypt(cipher, "AAAA-", FailureLog::Error, FailureValue::Original), Some("AAAA-"), &[Level::Debug])]
#[case::explicit_key_probe_empty_rejected(None, LEGACY, |_: &Cipher| probe_with("", "salt-one").or_else(|| probe_with("*", "salt-one")), None, &[])]
#[case::explicit_key_probe_matches(None, LEGACY, |_: &Cipher| probe_with(AES_HELLO, "salt-one"), Some("hello"), &[])]
#[case::explicit_key_probe_wrong_key(SALT, LEGACY, |_: &Cipher| probe_with(LEGACY_HELLO, "other"), None, &[])]
#[case::bearer_aad(SALT, LEGACY, |cipher: &Cipher| decrypt_bearer_token(cipher, BEARER, "litellm_"), None, &[])]
#[case::bearer_reads_python(SALT, LEGACY, |cipher: &Cipher| decrypt_bearer_token(cipher, BEARER, PREFIX), Some("payload"), &[])]
#[case::bearer_unpadded_base64(SALT, LEGACY, |cipher: &Cipher| encrypt_bearer_token(cipher, "payload", PREFIX).ok(), Some(BEARER), &[])]
#[case::bearer_either_alphabet(SALT, LEGACY, |cipher: &Cipher| decrypt_bearer_token(cipher, &BEARER.replace('-', "+"), PREFIX), Some("payload"), &[])]
#[case::bearer_missing_secret(None, AES, |cipher: &Cipher| encrypt_bearer_token(cipher, "payload", PREFIX).err().map(|error| error.to_string()), Some("Set LITELLM_SALT_KEY or a master key to mint bearer tokens"), &[])]
#[case::bearer_missing_secret_reads(None, AES, |cipher: &Cipher| decrypt_bearer_token(cipher, BEARER, PREFIX), None, &[])]
#[case::bearer_bad_prefix_base64_length_tag_utf8(SALT, LEGACY, bad_bearers, None, &[])]
#[case::bearer_error_silent(SALT, LEGACY, |cipher: &Cipher| decrypt_bearer_token(cipher, &format!("{PREFIX}AAAA"), PREFIX), None, &[])]
fn wire_interop(
    #[case] key: Option<&str>,
    #[case] algorithm: Algorithm,
    #[case] probe: fn(&Cipher) -> Option<String>,
    #[case] expected: Option<&str>,
    #[case] levels: &[Level],
) {
    let cipher = build(key, algorithm);
    assert_eq!(probe(&cipher).as_deref(), expected);
    let recorded: Vec<Level> = logs(&cipher).iter().map(|(level, _)| *level).collect();
    assert_eq!(recorded, levels);
}

#[rstest]
#[case::empty_map_unencrypted(LEGACY, |cipher: &Cipher| encrypt_secret_map(cipher, &BTreeMap::new(), None).unwrap(), "{}", 0)]
#[case::whole_map_encrypted_once(AES, |cipher: &Cipher| encrypt_secret_map(cipher, &BTreeMap::from([("a".to_owned(), "1".to_owned())]), None).unwrap(), MAP_AES, 0)]
#[case::python_ordered_map(LEGACY, |cipher: &Cipher| decode_text(cipher, MAP_PYTHON_ORDER), r#"{"a":"1","b":"2"}"#, 0)]
#[case::plaintext_map(LEGACY, |cipher: &Cipher| decode(cipher, serde_json::from_str(r#"{"a":"1"}"#).ok().as_ref()), r#"{"a":"1"}"#, 0)]
#[case::json_plaintext_map(LEGACY, |cipher: &Cipher| decode_text(cipher, r#"{"a":"1"}"#), r#"{"a":"1"}"#, 0)]
#[case::json_quoted_ciphertext(LEGACY, |cipher: &Cipher| decode_text(cipher, MAP_AES), r#"{"a":"1"}"#, 0)]
#[case::raw_ciphertext(LEGACY, |cipher: &Cipher| decode_text(cipher, MAP_AES.trim_matches('"')), r#"{"a":"1"}"#, 0)]
#[case::decrypted_empty_map(LEGACY, |cipher: &Cipher| decode_text(cipher, EMPTY_MAP_CIPHER), "{}", 0)]
#[case::leading_whitespace(LEGACY, |cipher: &Cipher| decode_text(cipher, " \n{\"a\":\"1\",\"a\":\"2\"}"), r#"{"a":"2"}"#, 0)]
#[case::unicode_whitespace_rejected(LEGACY, |cipher: &Cipher| decode_text(cipher, " \u{2003}{\"a\":\"1\"}"), REJECTED, 0)]
#[case::python_separator_whitespace(LEGACY, |cipher: &Cipher| decode_text(cipher, "\u{1c}{\"a\":\"1\"}"), REJECTED, 0)]
#[case::null_map(LEGACY, |cipher: &Cipher| decode(cipher, None), "null", 0)]
#[case::wrong_value_type(LEGACY, |cipher: &Cipher| decode(cipher, serde_json::from_str(r#"{"a":1}"#).ok().as_ref()), REJECTED, 0)]
#[case::quoted_garbage(LEGACY, |cipher: &Cipher| decode_text(cipher, r#""a" "#), REJECTED, 1)]
#[case::malformed_json(LEGACY, |cipher: &Cipher| decode_text(cipher, "{bad"), REJECTED, 0)]
#[case::bad_ciphertext_error_redaction(LEGACY, |cipher: &Cipher| decode_text(cipher, "sk-live-secret"), REJECTED, 1)]
#[case::decrypted_non_map(LEGACY, |cipher: &Cipher| decode_text(cipher, LEGACY_HELLO), REJECTED, 0)]
fn secret_maps(
    #[case] algorithm: Algorithm,
    #[case] probe: fn(&Cipher) -> String,
    #[case] expected: &str,
    #[case] debug_logs: usize,
) {
    let cipher = build(SALT, algorithm);
    assert_eq!(probe(&cipher), expected);
    let recorded = logs(&cipher);
    assert_eq!(recorded.len(), debug_logs);
    assert!(
        recorded
            .iter()
            .all(|(level, message)| *level == Level::Debug && !message.contains("sk-live"))
    );
}

#[test]
fn native_secret_diagnostics_redact_values() {
    let cipher = build(SALT, LEGACY);
    let structured = BTreeMap::from([("api_key".to_owned(), "sk-live-secret".to_owned())]);
    let passed = cipher
        .encrypt_value(StoredValue::Other(structured.clone()), None)
        .unwrap();
    assert_eq!(passed, StoredValue::Other(structured));
    assert_eq!(loud(&cipher, "sk-live-secret"), None);
    let stored = StoredMap::Text("sk-live-secret".to_owned());
    let error = decode_secret_map(&cipher, Some(&stored), "headers").unwrap_err();
    let messages: Vec<String> = logs(&cipher)
        .into_iter()
        .map(|(_, message)| message)
        .collect();
    assert_eq!(messages.len(), 4);
    assert!(messages[0].contains("BTreeMap"));
    assert!(messages[2].contains("api_key") && messages[2].contains("salt key change recently"));
    assert!(
        messages[2].ends_with("https://docs.litellm.ai/docs/proxy/prod#5-set-litellm-salt-key")
    );
    let rendered = format!("{messages:?} {error} {error:?}");
    assert!(!rendered.contains("sk-live"));
}

fn padded(value: &str, filler: &str, length: usize) -> String {
    format!(
        "{}{value}",
        filler.repeat(length.checked_sub(value.len()).unwrap())
    )
}

#[rstest]
#[case::scalar_at_limit(|cipher: &Cipher| loud(cipher, &padded(LEGACY_HELLO, "*", MAX_ENCODED_BYTES)), Some("hello"), 0)]
#[case::scalar_over_limit(|cipher: &Cipher| loud(cipher, &padded(LEGACY_HELLO, "*", MAX_ENCODED_BYTES.checked_add(1).unwrap())), None, 2)]
#[case::explicit_key_over_limit(|_: &Cipher| probe_with(&padded(LEGACY_HELLO, "*", MAX_ENCODED_BYTES.checked_add(1).unwrap()), "salt-one"), None, 0)]
#[case::bearer_over_limit(|cipher: &Cipher| decrypt_bearer_token(cipher, &padded("", "A", MAX_ENCODED_BYTES.checked_add(4).unwrap()), ""), None, 0)]
#[case::map_at_limit(|cipher: &Cipher| Some(decode_text(cipher, &padded("{}", " ", MAX_ENCODED_BYTES))), Some("{}"), 0)]
#[case::map_over_limit(|cipher: &Cipher| Some(decode_text(cipher, &padded("{}", " ", MAX_ENCODED_BYTES.checked_add(1).unwrap()))), Some(REJECTED), 0)]
#[case::map_depth(|cipher: &Cipher| Some(decode_text(cipher, &format!("{{\"a\":{}", "[".repeat(65)))), Some(REJECTED), 0)]
fn secret_input_limits(
    #[case] probe: fn(&Cipher) -> Option<String>,
    #[case] expected: Option<&str>,
    #[case] log_count: usize,
) {
    let cipher = build(SALT, LEGACY);
    assert_eq!(probe(&cipher).as_deref(), expected);
    assert_eq!(logs(&cipher).len(), log_count);
}

#[test]
fn replays_the_secret_value_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/secret_value");
    let cipher = build(SALT, AES);
    let seeds: Vec<Vec<u8>> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| fs::read(entry.unwrap().path()).unwrap())
        .collect();
    let opened = seeds
        .iter()
        .filter_map(|seed| str::from_utf8(seed).ok())
        .filter(|value| {
            let stored: Option<StoredMap> = serde_json::from_str(value).ok();
            let map = decode_secret_map(&cipher, stored.as_ref(), "corpus");
            let bearer = decrypt_bearer_token(&cipher, value, PREFIX);
            let scalar = loud(&cipher, value);
            map.is_ok_and(|decoded| decoded.is_some())
                | bearer.is_some()
                | scalar.is_some()
                | probe_with(value, "salt-one").is_some()
        });
    assert_eq!((seeds.len(), opened.count()), (10, 5));
}
