#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::{
    CachedValue, DecodeError, INCREMENT_WITH_FLOOR_LUA, decode_cached_value, decoded_counts,
    validate_lua_count,
};

#[test]
fn increment_with_floor_lua_matches_litellm_script() {
    assert_eq!(
        INCREMENT_WITH_FLOOR_LUA,
        "local count = redis.call('INCRBY', KEYS[1], ARGV[1]) \
if count < 0 then count = redis.call('INCRBY', KEYS[1], -count) end \
if redis.call('TTL', KEYS[1]) < 0 then redis.call('EXPIRE', KEYS[1], ARGV[2]) end \
return count"
    );
}

#[test]
fn decoded_counts_maps_bytes_and_none_to_optional_ints() {
    let values = [Some(b"3".to_vec()), None, Some(b"10".to_vec())];
    assert_eq!(decoded_counts(&values).unwrap(), [Some(3), None, Some(10)]);
}

#[test]
fn decoded_counts_rejects_non_numeric_bytes() {
    let values = [Some(b"not-a-number".to_vec())];
    assert!(matches!(
        decoded_counts(&values).unwrap_err(),
        DecodeError::Count { text, .. } if text == "not-a-number"
    ));
}

#[test]
fn decoded_counts_rejects_invalid_utf8() {
    let values = [Some(vec![0xFF, 0xFE])];
    assert!(matches!(
        decoded_counts(&values).unwrap_err(),
        DecodeError::Utf8 { .. }
    ));
}

#[test]
fn validate_lua_count_parses_a_single_reply() {
    assert_eq!(validate_lua_count(b"42").unwrap(), 42);
}

#[test]
fn decode_cached_value_passes_through_none() {
    assert_eq!(decode_cached_value(None).unwrap(), None);
}

#[test]
fn decode_cached_value_prefers_json() {
    let value = decode_cached_value(Some(br#"{"a": 1}"#)).unwrap().unwrap();
    let expected = CachedValue::Object(BTreeMap::from([(
        "a".to_owned(),
        CachedValue::Number(serde_json::Number::from(1_i32)),
    )]));
    assert_eq!(value, expected);
}

#[test]
fn decode_cached_value_falls_back_to_python_literal() {
    let value = decode_cached_value(Some(b"{'a': 1, 'b': [1, 2, True, None]}"))
        .unwrap()
        .unwrap();
    let expected = CachedValue::Object(BTreeMap::from([
        (
            "a".to_owned(),
            CachedValue::Number(serde_json::Number::from(1_i32)),
        ),
        (
            "b".to_owned(),
            CachedValue::Array(vec![
                CachedValue::Number(serde_json::Number::from(1_i32)),
                CachedValue::Number(serde_json::Number::from(2_i32)),
                CachedValue::Boolean(true),
                CachedValue::Null,
            ]),
        ),
    ]));
    assert_eq!(value, expected);
}

#[test]
fn decode_cached_value_propagates_when_both_decodes_fail() {
    assert!(matches!(
        decode_cached_value(Some(b"not json and not a literal")).unwrap_err(),
        DecodeError::LiteralEval { .. }
    ));
}

#[test]
fn decode_cached_value_rejects_invalid_utf8() {
    assert!(matches!(
        decode_cached_value(Some(&[0xFF, 0xFE])).unwrap_err(),
        DecodeError::Utf8 { .. }
    ));
}

#[rstest]
#[case::bytes_literal(b"b'abc'".as_slice())]
#[case::complex_literal(b"3+4j".as_slice())]
fn decode_cached_value_rejects_literals_without_json_shape(#[case] text: &[u8]) {
    assert!(matches!(
        decode_cached_value(Some(text)).unwrap_err(),
        DecodeError::UnsupportedLiteral
    ));
}

#[test]
fn decode_cached_value_rejects_non_string_dict_keys() {
    assert!(matches!(
        decode_cached_value(Some(b"{1: 'a'}")).unwrap_err(),
        DecodeError::NonStringKey
    ));
}
