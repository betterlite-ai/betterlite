#![cfg(test)]

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_secrets::secret_codec::OsNonces;
use betterlite_secrets::stored_cipher::app_crypto::{AppCrypto, Envelope};
use rstest::rstest;
use secrecy::{SecretSlice, SecretString};
use serde::Deserialize;

use crate::fields::payload::StandardLoggingPayload;
use crate::sqs_logging::encryption::Encryption;
use crate::sqs_logging::tests::support::payload;

use super::{form_body, message_json};

const KEY_BYTES: [u8; 32] = [7; 32];

#[derive(Deserialize)]
struct Wrapped {
    #[serde(rename = "__encrypted__")]
    encrypted: bool,
    payload: Envelope,
}

fn key() -> SecretString {
    SecretString::from(STANDARD.encode(KEY_BYTES))
}

fn decrypted_matches(wrapped: &Wrapped, aad: &[u8], expected: &StandardLoggingPayload) -> bool {
    let app = AppCrypto::new(&SecretSlice::from(KEY_BYTES.to_vec()), OsNonces)
        .expect("the key is 32 bytes");
    match app.decrypt_json(&wrapped.payload, aad) {
        Ok(decrypted) => {
            decrypted == serde_json::to_value(expected).expect("the payload serializes")
        }
        Err(_error) => false,
    }
}

#[rstest]
#[case::json_object(r#"{"a":"b"}"#, "%7B%22a%22%3A%22b%22%7D")]
#[case::ampersand_and_equals("a&b=c", "a%26b%3Dc")]
#[case::plus_and_space("+ /", "%2B%20%2F")]
#[case::unreserved("AZaz09_.-~", "AZaz09_.-~")]
#[case::unicode("\u{e9}", "%C3%A9")]
fn form_body_percent_encodes_the_message_body(#[case] message: &str, #[case] encoded: &str) {
    assert_eq!(
        form_body(message),
        format!("Action=SendMessage&Version=2012-11-05&MessageBody={encoded}")
    );
}

#[test]
fn message_json_serializes_the_standard_payload() {
    let json = message_json(&payload(), None).expect("the payload serializes");
    assert!(json.contains(r#""id":"sqs-health-check""#));
    assert!(json.contains(r#""model":"gpt-3.5-turbo""#));
}

#[test]
fn message_json_encrypts_with_the_encrypted_marker() {
    let encryption = Encryption::from_base64(&key(), None).expect("the key is valid");
    let payload = payload();
    let json = message_json(&payload, Some(&encryption)).expect("the payload encrypts");
    let wrapped: Wrapped = serde_json::from_str(&json).expect("the envelope is JSON");
    assert!(wrapped.encrypted);
    assert!(!json.contains("gpt-3.5-turbo"));
    assert!(decrypted_matches(&wrapped, b"", &payload));
}

#[test]
fn message_json_encrypts_with_the_configured_aad() {
    let encryption =
        Encryption::from_base64(&key(), Some("tenant=bill".to_owned())).expect("valid key");
    let payload = payload();
    let json = message_json(&payload, Some(&encryption)).expect("the payload encrypts");
    let wrapped: Wrapped = serde_json::from_str(&json).expect("the envelope is JSON");
    assert!(decrypted_matches(&wrapped, b"tenant=bill", &payload));
    assert!(!decrypted_matches(&wrapped, b"tenant=alice", &payload));
}

#[test]
fn message_json_without_aad_decrypts_with_an_empty_aad() {
    let encryption = Encryption::from_base64(&key(), None).expect("the key is valid");
    let payload = payload();
    let json = message_json(&payload, Some(&encryption)).expect("the payload encrypts");
    let wrapped: Wrapped = serde_json::from_str(&json).expect("the envelope is JSON");
    assert!(decrypted_matches(&wrapped, b"", &payload));
}
