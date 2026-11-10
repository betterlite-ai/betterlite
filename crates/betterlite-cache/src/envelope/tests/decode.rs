#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use betterlite_context::local_ttl_cache::CacheValue;
use rstest::rstest;

use super::support_tests::{NOW, envelope, reply_response};
use crate::envelope::decode::{DecodeError, decode_cached, parse_cached_form};
use crate::envelope::{RESPONSE_FIELD, TIMESTAMP_FIELD};

const FRESH: f64 = 9_999_999_999.0;

fn stored_envelope() -> CacheValue {
    envelope(Some(FRESH), reply_response())
}

fn decode_bytes(bytes: &[u8]) -> Result<Option<CacheValue>, DecodeError> {
    let cached = parse_cached_form(bytes)?;
    decode_cached(Some(&cached), None, NOW)
}

#[test]
fn cached_envelope_returns_its_response() {
    let cached = stored_envelope();
    assert_eq!(
        decode_cached(Some(&cached), Some(60), NOW).expect("a decoded response"),
        Some(reply_response())
    );
}

#[test]
fn cached_object_without_timestamp_returns_as_is() {
    let cached = reply_response();
    assert_eq!(
        decode_cached(Some(&cached), Some(60), NOW).expect("the cached object"),
        Some(reply_response())
    );
}

#[rstest]
#[case::text(CacheValue::Text("cached".to_owned()))]
#[case::integer(CacheValue::Integer(7))]
#[case::array(CacheValue::Array(vec![CacheValue::Integer(7)]))]
fn cached_non_object_returns_as_is(#[case] cached: CacheValue) {
    assert_eq!(
        decode_cached(Some(&cached), None, NOW).expect("the cached value"),
        Some(cached)
    );
}

#[test]
fn missing_cached_value_is_a_miss() {
    assert!(matches!(decode_cached(None, Some(60), NOW), Ok(None)));
}

#[test]
fn response_age_equal_to_max_age_is_fresh() {
    let cached = envelope(Some(40.0_f64), reply_response());
    assert_eq!(
        decode_cached(Some(&cached), Some(60), 100.0_f64).expect("a fresh response"),
        Some(reply_response())
    );
}

#[test]
fn response_age_one_step_past_max_age_is_stale() {
    let cached = envelope(Some(39.9_f64), reply_response());
    assert!(matches!(
        decode_cached(Some(&cached), Some(60), 100.0_f64),
        Ok(None)
    ));
}

#[test]
fn response_age_one_step_under_max_age_is_fresh() {
    let cached = envelope(Some(40.1_f64), reply_response());
    assert_eq!(
        decode_cached(Some(&cached), Some(60), 100.0_f64).expect("a fresh response"),
        Some(reply_response())
    );
}

#[test]
fn absent_max_age_accepts_an_old_response() {
    let cached = envelope(Some(0.0_f64), reply_response());
    assert_eq!(
        decode_cached(Some(&cached), None, NOW).expect("an unbounded response"),
        Some(reply_response())
    );
}

#[test]
fn integer_timestamp_is_read_as_seconds() {
    let mut fields = BTreeMap::new();
    fields.insert(TIMESTAMP_FIELD.to_owned(), CacheValue::Integer(40));
    fields.insert(RESPONSE_FIELD.to_owned(), reply_response());
    let cached = CacheValue::Object(fields);
    assert_eq!(
        decode_cached(Some(&cached), Some(60), 100.0_f64).expect("a fresh response"),
        Some(reply_response())
    );
}

#[test]
fn non_numeric_timestamp_is_an_error() {
    let mut fields = BTreeMap::new();
    fields.insert(
        TIMESTAMP_FIELD.to_owned(),
        CacheValue::Text("soon".to_owned()),
    );
    fields.insert(RESPONSE_FIELD.to_owned(), reply_response());
    let cached = CacheValue::Object(fields);
    assert!(matches!(
        decode_cached(Some(&cached), None, NOW),
        Err(DecodeError::Timestamp)
    ));
}

#[test]
fn envelope_without_response_is_an_error() {
    let mut fields = BTreeMap::new();
    fields.insert(TIMESTAMP_FIELD.to_owned(), CacheValue::Float(FRESH));
    let cached = CacheValue::Object(fields);
    assert!(matches!(
        decode_cached(Some(&cached), None, NOW),
        Err(DecodeError::MissingResponse)
    ));
}

#[test]
fn json_string_response_is_decoded() {
    // null is JSON-only: the bounded literal parser rejects it, so only the JSON step can decode this
    let cached = envelope(
        Some(FRESH),
        CacheValue::Text("{\"choices\": [], \"usage\": null}".to_owned()),
    );
    let expected = CacheValue::Object(BTreeMap::from([
        ("choices".to_owned(), CacheValue::Array(Vec::new())),
        ("usage".to_owned(), CacheValue::Null),
    ]));
    assert_eq!(
        decode_cached(Some(&cached), None, NOW).expect("a decoded response"),
        Some(expected)
    );
}

#[test]
fn literal_string_response_falls_back_to_the_bounded_parser() {
    let cached = envelope(
        Some(FRESH),
        CacheValue::Text("{'choices': [], 'usage': None}".to_owned()),
    );
    let expected = CacheValue::Object(BTreeMap::from([
        ("choices".to_owned(), CacheValue::Array(Vec::new())),
        ("usage".to_owned(), CacheValue::Null),
    ]));
    assert_eq!(
        decode_cached(Some(&cached), None, NOW).expect("a decoded literal"),
        Some(expected)
    );
}

#[test]
fn unparsable_string_response_is_an_error() {
    let cached = envelope(
        Some(FRESH),
        CacheValue::Text("__import__('os').system('true')".to_owned()),
    );
    assert!(matches!(
        decode_cached(Some(&cached), None, NOW),
        Err(DecodeError::Form { .. })
    ));
}

#[test]
fn bytes_response_is_decoded() {
    let stored = br#"{"choices": []}"#.to_vec();
    let cached = envelope(Some(FRESH), CacheValue::Bytes(stored));
    assert_eq!(
        decode_cached(Some(&cached), None, NOW).expect("a decoded response"),
        Some(reply_response())
    );
}

#[rstest]
#[case::number(CacheValue::Integer(1))]
#[case::flag(CacheValue::Boolean(true))]
#[case::list(CacheValue::Array(Vec::new()))]
#[case::null(CacheValue::Null)]
fn non_text_non_object_response_is_an_error(#[case] response: CacheValue) {
    let cached = envelope(Some(FRESH), response);
    assert!(matches!(
        decode_cached(Some(&cached), None, NOW),
        Err(DecodeError::ResponseType)
    ));
}

#[test]
fn cached_bytes_read_a_json_envelope() {
    // lowercase true is JSON-only: the bounded literal parser accepts True, not true
    let bytes = br#"{"timestamp": 9999999999, "response": {"stream": true}}"#;
    let expected = CacheValue::Object(BTreeMap::from([(
        "stream".to_owned(),
        CacheValue::Boolean(true),
    )]));
    assert_eq!(
        decode_bytes(bytes).expect("a decoded envelope"),
        Some(expected)
    );
}

#[test]
fn cached_bytes_read_a_literal_envelope() {
    let bytes = b"{'timestamp': 9999999999, 'response': {'choices': []}}";
    assert_eq!(
        decode_bytes(bytes).expect("a decoded literal envelope"),
        Some(reply_response())
    );
}

#[test]
fn cached_bytes_reject_garbage() {
    assert!(matches!(
        decode_bytes(b"not a literal"),
        Err(DecodeError::Form { .. })
    ));
    assert!(matches!(
        decode_bytes(&[0xff, 0xfe]),
        Err(DecodeError::Form { .. })
    ));
}

#[rstest]
fn replays_the_cache_envelope_decoder_corpus(
    #[files("fuzz/corpus/cache_envelope_decoder-*")] path: PathBuf,
) {
    let bytes = fs::read(&path).expect("a corpus seed");
    let name = path
        .file_name()
        .and_then(|file_name| file_name.to_str())
        .expect("a seed name")
        .to_owned();
    let outcome = decode_bytes(&bytes);
    if name.contains("invalid") {
        let _error = outcome.expect_err("an invalid seed must not decode");
    } else {
        let _decoded = outcome.expect("a valid seed must decode");
    }
}
