#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;
use std::str;
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use base64::engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tracing::Level as TraceLevel;
use tracing::subscriber::with_default;

use crate::error::SecretError;
use crate::secret_codec::bearer::{decrypt_bearer_token, encrypt_bearer_token};
use crate::secret_codec::diagnostics::{Level, SecretDiagnostics, SecretEvent, TracingDiagnostics};
use crate::secret_codec::python_base64::decode_strict;
use crate::secret_codec::sealing::{open, seal, seal_legacy};
use crate::secret_codec::secret_map::{decode_secret_map, encrypt_secret_map};
use crate::secret_codec::{
    Algorithm, CipherSettings, DecryptPolicy, FailureLog, FailureValue, MAX_ENCODED_BYTES,
    NonceSource, OsNonces, StoredCipher, StoredValue, decrypt_if_encrypted_with,
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

#[rstest]
#[case::four_trailing_bits("Zh==", Some("f"))]
#[case::two_trailing_bits("aGl=", Some("hi"))]
#[case::unpadded_four_bits("Zh", Some("f"))]
#[case::unpadded_two_bits("aGl", Some("hi"))]
#[case::canonical("Zg==", Some("f"))]
#[case::mime_whitespace("Z\r\nh==", None)]
#[case::padded_concatenation("Zh==Zm8=", None)]
#[case::excess_padding("Zh===", None)]
#[case::nonalphabet("Zh*=", None)]
#[case::short_group("Z===", None)]
fn bearer_base64_acceptance(#[case] input: &str, #[case] expected: Option<&str>) {
    let decoded = decode_strict(input).ok();
    assert_eq!(decoded.as_deref(), expected.map(str::as_bytes));
}

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

fn exposed(text: Option<SecretString>) -> Option<String> {
    Some(text?.expose_secret().to_owned())
}

fn encrypt(cipher: &Cipher, value: &str, override_key: Option<&str>) -> Option<String> {
    let key = override_key.map(SecretString::from);
    let text = StoredValue::<()>::Text(SecretString::from(value));
    match cipher.encrypt_value(text, key.as_ref()) {
        Ok(StoredValue::Text(sealed)) => exposed(Some(sealed)),
        Ok(StoredValue::Other(())) => None,
        Err(error) => Some(format!("error: {error}")),
    }
}

fn decrypt(cipher: &Cipher, text: &str, log: FailureLog, value: FailureValue) -> Option<String> {
    let stored = StoredValue::<()>::Text(SecretString::from(text));
    match cipher.decrypt_value(stored, "api_key", DecryptPolicy { log, value }) {
        Some(StoredValue::Text(opened)) => exposed(Some(opened)),
        Some(StoredValue::Other(())) | None => None,
    }
}

fn loud(cipher: &Cipher, value: &str) -> Option<String> {
    decrypt(cipher, value, FailureLog::Error, FailureValue::Nothing)
}

fn probe_with(value: &str, key: &str) -> Option<String> {
    exposed(decrypt_if_encrypted_with(value, &SecretString::from(key)))
}

fn open_bearer(cipher: &Cipher, token: &str, prefix: &str) -> Option<String> {
    exposed(decrypt_bearer_token(cipher, token, prefix))
}

fn decode(cipher: &Cipher, stored: Option<&str>) -> String {
    match decode_secret_map(cipher, stored, "headers") {
        Ok(map) => {
            let plain = map.map(|entries| {
                entries
                    .into_iter()
                    .map(|(name, text)| (name, exposed(Some(text))))
            });
            serde_json::to_string(&plain.map(BTreeMap::from_iter)).unwrap()
        }
        Err(error) => format!("error: {error}"),
    }
}

fn decode_text(cipher: &Cipher, value: &str) -> String {
    decode(cipher, Some(&serde_json::to_string(value).unwrap()))
}

fn utf8_bearer(cipher: &Cipher) -> Option<String> {
    let key = SecretString::from("salt-one");
    let sealed = seal(AES, &key, &CountingNonces, &[0xff], PREFIX.as_bytes()).unwrap();
    let token = format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(sealed));
    open_bearer(cipher, &token, PREFIX)
}

fn bad_bearers(cipher: &Cipher) -> Option<String> {
    let wrong_prefix = open_bearer(cipher, BEARER, "other_");
    let alphabet = open_bearer(cipher, &BEARER.replace('-', "*"), PREFIX);
    let length = open_bearer(cipher, &format!("{BEARER}AA"), PREFIX);
    let tag = open_bearer(cipher, &BEARER.replace('Q', "R"), PREFIX);
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
#[case::permissive_stray_bits(SALT, LEGACY, |cipher: &Cipher| loud(cipher, &LONG_STANDARD.replace("hno=", "hnp=")), Some("h\u{e9}llo w\u{f6}rld, a longer secret value!"), &[])]
#[case::bearer_stray_bits(SALT, LEGACY, |cipher: &Cipher| open_bearer(cipher, &BEARER.replace("Vzw8", "Vzw9"), PREFIX), Some("payload"), &[])]
#[case::empty_legacy_bytes(SALT, LEGACY, |cipher: &Cipher| loud(cipher, "*"), Some(""), &[])]
#[case::legacy_box_seals_python_bytes(None, LEGACY, |_: &Cipher| seal_legacy(&SecretString::from("hello"), &SecretString::from("salt-one"), &CountingNonces).ok().map(|sealed| URL_SAFE.encode(sealed)), Some(LEGACY_HELLO), &[])]
#[case::legacy_box_opens_python_bytes(None, LEGACY, |_: &Cipher| exposed(open(Algorithm::XSalsa20Poly1305, &SecretString::from("salt-one"), &URL_SAFE.decode(LEGACY_HELLO).unwrap(), b"").ok()), Some("hello"), &[])]
#[case::legacy_box_wrong_key(None, LEGACY, |_: &Cipher| exposed(open(Algorithm::XSalsa20Poly1305, &SecretString::from("other"), &URL_SAFE.decode(LEGACY_HELLO).unwrap(), b"").ok()), None, &[])]
#[case::plaintext_debug_redacted(SALT, LEGACY, |cipher: &Cipher| Some(format!("{:?} {:?}", cipher.decrypt_value(StoredValue::<()>::Text(SecretString::from(LEGACY_HELLO)), "k", DecryptPolicy { log: FailureLog::Debug, value: FailureValue::Original }), decrypt_bearer_token(cipher, BEARER, PREFIX))), Some("Some(Text(SecretBox<str>([REDACTED]))) Some(SecretBox<str>([REDACTED]))"), &[])]
#[case::explicit_key_probe_debug_redacted(None, LEGACY, |_: &Cipher| Some(format!("{:?}", decrypt_if_encrypted_with(AES_HELLO, &SecretString::from("salt-one")))), Some("Some(SecretBox<str>([REDACTED]))"), &[])]
#[case::legacy_box_empty_buffer(None, LEGACY, |_: &Cipher| exposed(open(Algorithm::XSalsa20Poly1305, &SecretString::from("other"), &[], b"").ok()), Some(""), &[])]
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
#[case::bearer_aad(SALT, LEGACY, |cipher: &Cipher| open_bearer(cipher, BEARER, "litellm_"), None, &[])]
#[case::bearer_reads_python(SALT, LEGACY, |cipher: &Cipher| open_bearer(cipher, BEARER, PREFIX), Some("payload"), &[])]
#[case::bearer_unpadded_base64(SALT, LEGACY, |cipher: &Cipher| encrypt_bearer_token(cipher, &SecretString::from("payload"), PREFIX).ok(), Some(BEARER), &[])]
#[case::bearer_either_alphabet(SALT, LEGACY, |cipher: &Cipher| open_bearer(cipher, &BEARER.replace('-', "+"), PREFIX), Some("payload"), &[])]
#[case::bearer_missing_secret(None, AES, |cipher: &Cipher| encrypt_bearer_token(cipher, &SecretString::from("payload"), PREFIX).err().map(|error| error.to_string()), Some("Set LITELLM_SALT_KEY or a master key to mint bearer tokens"), &[])]
#[case::bearer_missing_secret_reads(None, AES, |cipher: &Cipher| open_bearer(cipher, BEARER, PREFIX), None, &[])]
#[case::bearer_bad_prefix_base64_length_tag_utf8(SALT, LEGACY, bad_bearers, None, &[])]
#[case::bearer_error_silent(SALT, LEGACY, |cipher: &Cipher| open_bearer(cipher, &format!("{PREFIX}AAAA"), PREFIX), None, &[])]
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
#[case::whole_map_encrypted_once(AES, |cipher: &Cipher| encrypt_secret_map(cipher, &BTreeMap::from([("a".to_owned(), SecretString::from("1"))]), None).unwrap(), MAP_AES, 0)]
#[case::python_ordered_map(LEGACY, |cipher: &Cipher| decode_text(cipher, MAP_PYTHON_ORDER), r#"{"a":"1","b":"2"}"#, 0)]
#[case::plaintext_map(LEGACY, |cipher: &Cipher| decode(cipher, Some(r#"{"a":"1"}"#)), r#"{"a":"1"}"#, 0)]
#[case::json_plaintext_map(LEGACY, |cipher: &Cipher| decode_text(cipher, r#"{"a":"1"}"#), r#"{"a":"1"}"#, 0)]
#[case::json_quoted_ciphertext(LEGACY, |cipher: &Cipher| decode_text(cipher, MAP_AES), r#"{"a":"1"}"#, 0)]
#[case::two_symbol_stray_bits(LEGACY, |cipher: &Cipher| decode_text(cipher, &MAP_AES.replace("wg==", "wh==")), r#"{"a":"1"}"#, 0)]
#[case::raw_ciphertext(LEGACY, |cipher: &Cipher| decode_text(cipher, MAP_AES.trim_matches('"')), r#"{"a":"1"}"#, 0)]
#[case::decrypted_empty_map(LEGACY, |cipher: &Cipher| decode_text(cipher, EMPTY_MAP_CIPHER), "{}", 0)]
#[case::leading_whitespace(LEGACY, |cipher: &Cipher| decode_text(cipher, " \n{\"a\":\"1\",\"a\":\"2\"}"), r#"{"a":"2"}"#, 0)]
#[case::unicode_whitespace_rejected(LEGACY, |cipher: &Cipher| decode_text(cipher, " \u{2003}{\"a\":\"1\"}"), REJECTED, 0)]
#[case::python_separator_whitespace(LEGACY, |cipher: &Cipher| decode_text(cipher, "\u{1c}{\"a\":\"1\"}"), REJECTED, 0)]
#[case::null_map(LEGACY, |cipher: &Cipher| decode(cipher, None), "null", 0)]
#[case::stored_null(LEGACY, |cipher: &Cipher| decode(cipher, Some("null")), "null", 0)]
#[case::stored_empty_map(LEGACY, |cipher: &Cipher| decode(cipher, Some("{}")), "{}", 0)]
#[case::stored_invalid_json(LEGACY, |cipher: &Cipher| decode(cipher, Some("{bad")), REJECTED, 0)]
#[case::wrong_value_type(LEGACY, |cipher: &Cipher| decode(cipher, Some(r#"{"a":1}"#)), REJECTED, 0)]
#[case::quoted_garbage(LEGACY, |cipher: &Cipher| decode_text(cipher, r#""a" "#), REJECTED, 1)]
#[case::malformed_json(LEGACY, |cipher: &Cipher| decode_text(cipher, "{bad"), REJECTED, 0)]
#[case::bad_ciphertext_error_redaction(LEGACY, |cipher: &Cipher| decode_text(cipher, "sk-live-secret"), REJECTED, 1)]
#[case::decoded_map_debug_redacted(LEGACY, |cipher: &Cipher| format!("{:?}", decode_secret_map(cipher, Some(MAP_AES), "headers")), r#"Ok(Some({"a": SecretBox<str>([REDACTED])}))"#, 0)]
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
    assert!(matches!(passed, StoredValue::Other(map) if map == structured));
    assert_eq!(loud(&cipher, "sk-live-secret"), None);
    let stored = SecretString::from(r#""sk-live-secret""#);
    let error = decode_secret_map(&cipher, Some(stored.expose_secret()), "headers").unwrap_err();
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
    assert_eq!(
        messages[2],
        "Error decrypting value for key: api_key, Did your master_key/salt key change recently? \nError: the secret is too short to hold its nonce\nSet permanent salt key - https://docs.litellm.ai/docs/proxy/prod#5-set-litellm-salt-key"
    );
    let rendered = format!("{messages:?} {error} {error:?} {stored:?}");
    assert!(!rendered.contains("sk-live"));
}
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn production_sources_reach_the_os_and_tracing() {
    let mut first = [0_u8; 24];
    let mut second = [0_u8; 24];
    OsNonces.fill(&mut first).unwrap();
    OsNonces.fill(&mut second).unwrap();
    assert_ne!(first, second);
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(TraceLevel::DEBUG)
        .with_writer(move || writer.clone())
        .finish();
    let event = SecretEvent::UnableToDecrypt { field: "api_key" };
    with_default(subscriber, || {
        TracingDiagnostics.record(Level::Debug, &event);
        TracingDiagnostics.record(Level::Error, &event);
    });
    let output = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    let unable = output.matches("Unable to decrypt value for key: api_key");
    assert_eq!(unable.count(), 2);
    assert!(output.contains("DEBUG") && output.contains("ERROR"));
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
#[case::bearer_over_limit(|cipher: &Cipher| open_bearer(cipher, &padded("", "A", MAX_ENCODED_BYTES.checked_add(4).unwrap()), ""), None, 0)]
#[case::map_at_limit(|cipher: &Cipher| Some(decode_text(cipher, &padded("{}", " ", MAX_ENCODED_BYTES.checked_sub(2).unwrap()))), Some("{}"), 0)]
#[case::map_over_limit(|cipher: &Cipher| Some(decode_text(cipher, &padded("{}", " ", MAX_ENCODED_BYTES.checked_add(1).unwrap()))), Some(REJECTED), 0)]
#[case::stored_map_at_limit(|cipher: &Cipher| Some(decode(cipher, Some(&padded("{}", " ", MAX_ENCODED_BYTES)))), Some("{}"), 0)]
#[case::stored_map_over_limit(|cipher: &Cipher| Some(decode(cipher, Some(&format!("{{\"a\":\"{}\"}}", "x".repeat(MAX_ENCODED_BYTES))))), Some(REJECTED), 0)]
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
            let map = decode_secret_map(&cipher, Some(value), "corpus");
            let text = serde_json::to_string(value).unwrap();
            let text_map = decode_secret_map(&cipher, Some(&text), "corpus");
            let bearer = decrypt_bearer_token(&cipher, value, PREFIX);
            let scalar = loud(&cipher, value);
            map.is_ok_and(|decoded| decoded.is_some())
                | text_map.is_ok_and(|decoded| decoded.is_some())
                | bearer.is_some()
                | scalar.is_some()
                | probe_with(value, "salt-one").is_some()
        });
    assert_eq!((seeds.len(), opened.count()), (10, 5));
}

#[rstest]
#[case::byte_limit("byte-limit.txt")]
#[case::depth_limit("depth-limit.json")]
fn corpus_map_limits_are_rejected(#[case] seed: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fuzz/corpus/secret_value")
        .join(seed);
    let value = fs::read_to_string(path).unwrap();
    let text = serde_json::to_string(&value).unwrap();
    let cipher = build(SALT, AES);
    assert!(matches!(
        decode_secret_map(&cipher, Some(&value), "headers"),
        Err(SecretError::MapDecode { .. })
    ));
    assert!(matches!(
        decode_secret_map(&cipher, Some(&text), "headers"),
        Err(SecretError::MapDecode { .. })
    ));
    assert!(logs(&cipher).is_empty());
}

#[rstest]
#[case::depth_limit(65)]
#[case::beyond_serde_limit(129)]
#[case::deeply_nested(4096)]
fn nested_maps_use_decode_error(#[case] depth: usize) {
    let raw = format!("{{\"a\":{}0{}}}", "[".repeat(depth), "]".repeat(depth));
    let text = serde_json::to_string(&raw).unwrap();
    let cipher = build(SALT, AES);
    assert!(matches!(
        decode_secret_map(&cipher, Some(&raw), "headers"),
        Err(SecretError::MapDecode { .. })
    ));
    assert!(matches!(
        decode_secret_map(&cipher, Some(&text), "headers"),
        Err(SecretError::MapDecode { .. })
    ));
    assert!(logs(&cipher).is_empty());
}
