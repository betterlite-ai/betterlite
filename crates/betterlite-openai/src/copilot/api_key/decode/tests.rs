#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;
use secrecy::ExposeSecret as _;
use serde_json::Value;

use super::decode_copilot_key_record;
use crate::copilot::device_auth::decode::{REPLY_LIMIT, ReplyError};

fn value(source: &str) -> Value {
    serde_json::from_str(source).unwrap()
}

#[test]
fn a_record_keeps_typed_fields_and_unknown_members() {
    let record = decode_copilot_key_record(
        br#"{"token":"key-value","expires_at":1800000000,"endpoints":{"api":"https://api.example","proxy":"https://proxy.example"},"sku":"copilot-pro"}"#,
    )
    .unwrap();

    assert_eq!(record.token.unwrap().expose_secret(), "key-value");
    assert_eq!(record.expires_at, Some(1_800_000_000));
    let endpoints = record.endpoints.unwrap();
    assert_eq!(endpoints.api.as_deref(), Some("https://api.example"));
    assert_eq!(
        endpoints.additional_fields.get("proxy"),
        Some(&value(r#""https://proxy.example""#))
    );
    assert_eq!(
        record.additional_fields.get("sku"),
        Some(&value(r#""copilot-pro""#))
    );
}

#[test]
fn a_record_without_the_modelled_fields_decodes_to_defaults() {
    let record = decode_copilot_key_record(b"{}").unwrap();

    assert!(record.token.is_none());
    assert!(record.expires_at.is_none());
    assert!(record.endpoints.is_none());
    assert!(record.additional_fields.is_empty());
}

#[test]
fn an_empty_token_stays_an_empty_token() {
    let record = decode_copilot_key_record(br#"{"token":""}"#).unwrap();

    assert_eq!(record.token.unwrap().expose_secret(), "");
}

#[test]
fn a_record_round_trips_through_its_serialized_form() {
    let source = value(
        r#"{"token":"key-value","expires_at":1800000000,"endpoints":{"api":"https://api.example","proxy":"https://proxy.example"},"sku":"copilot-pro"}"#,
    );
    let bytes = serde_json::to_vec(&source).unwrap();

    let record = decode_copilot_key_record(&bytes).unwrap();
    let written: Value = serde_json::from_slice(&serde_json::to_vec(&record).unwrap()).unwrap();

    assert_eq!(written, source);
}

#[test]
fn serializing_a_record_omits_absent_fields() {
    let record = decode_copilot_key_record(br#"{"expires_at":17}"#).unwrap();

    let written: Value = serde_json::from_slice(&serde_json::to_vec(&record).unwrap()).unwrap();

    assert_eq!(written, value(r#"{"expires_at":17}"#));
}

#[test]
fn a_null_modelled_field_is_written_back_absent() {
    let record =
        decode_copilot_key_record(br#"{"token":null,"expires_at":17,"endpoints":null}"#).unwrap();

    let written: Value = serde_json::from_slice(&serde_json::to_vec(&record).unwrap()).unwrap();

    assert!(record.token.is_none());
    assert!(record.endpoints.is_none());
    assert_eq!(written, value(r#"{"expires_at":17}"#));
}

#[rstest]
#[case::array(b"[]".as_slice())]
#[case::string(br#""token""#.as_slice())]
#[case::truncated(b"{".as_slice())]
#[case::not_json(b"not json".as_slice())]
fn malformed_records_are_rejected(#[case] bytes: &[u8]) {
    assert!(matches!(
        decode_copilot_key_record(bytes),
        Err(ReplyError::Json(_))
    ));
}

#[rstest]
#[case::numeric_token(br#"{"token":5}"#.as_slice())]
#[case::string_expiry(br#"{"token":"t","expires_at":"1800000000"}"#.as_slice())]
#[case::fractional_expiry(br#"{"token":"t","expires_at":1800000000.5}"#.as_slice())]
#[case::array_endpoints(br#"{"token":"t","endpoints":["https://api.example"]}"#.as_slice())]
#[case::numeric_endpoint(br#"{"token":"t","endpoints":{"api":5}}"#.as_slice())]
fn malformed_field_types_are_not_repaired(#[case] bytes: &[u8]) {
    assert!(matches!(
        decode_copilot_key_record(bytes),
        Err(ReplyError::Json(_))
    ));
}

#[test]
fn a_record_is_bounded_by_the_reply_limit() {
    let bounded = format!("{{}}{}", " ".repeat(REPLY_LIMIT - 2));
    decode_copilot_key_record(bounded.as_bytes()).unwrap();

    assert!(matches!(
        decode_copilot_key_record(format!("{bounded} ").as_bytes()),
        Err(ReplyError::Size { limit: REPLY_LIMIT })
    ));
}

#[rstest]
fn replays_the_copilot_key_corpus(#[files("fuzz/corpus/copilot_key/*")] path: PathBuf) {
    let bytes = fs::read(path).expect("corpus file");

    let _record = decode_copilot_key_record(&bytes);
}
