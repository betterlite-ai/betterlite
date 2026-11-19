use rstest::rstest;

use crate::auth_pubsub::channel::{INVALIDATION_CHANNEL, invalidation_channel};
use crate::auth_pubsub::decode::{
    DecodeError, MAX_MESSAGE_BYTES, MAX_MESSAGE_DEPTH, TypedValue, decode_invalidation,
};
use crate::auth_pubsub::message::{Invalidation, encode, read};

fn parsed(bytes: &[u8]) -> Option<Invalidation> {
    decode_invalidation(bytes).ok().as_ref().and_then(read)
}

fn nested_extra(brackets: usize) -> Vec<u8> {
    let prefix = "{\"cache_key\":\"k\",\"extra\":";
    let nested = format!("{}{}", "[".repeat(brackets), "]".repeat(brackets));
    format!("{prefix}{nested}}}").into_bytes()
}

fn nested_objects(levels: usize) -> Vec<u8> {
    let prefix = "{\"cache_key\":\"k\",\"extra\":";
    let open = "{\"a\":".repeat(levels);
    let close = "}".repeat(levels);
    format!("{prefix}{open}null{close}}}").into_bytes()
}

fn delete(cache_key: &str) -> Invalidation {
    Invalidation {
        cache_key: cache_key.to_owned(),
        new_value: None,
        ttl: None,
    }
}

#[test]
fn namespace_none_empty_present() {
    assert_eq!(
        INVALIDATION_CHANNEL,
        "litellm_proxy.auth_cache_invalidation"
    );
    assert_eq!(
        invalidation_channel(None),
        "litellm_proxy.auth_cache_invalidation"
    );
    assert_eq!(
        invalidation_channel(Some("")),
        ":litellm_proxy.auth_cache_invalidation"
    );
    assert_eq!(
        invalidation_channel(Some("tenant")),
        "tenant:litellm_proxy.auth_cache_invalidation"
    );
}

#[test]
fn json_omits_nulls() {
    assert_eq!(encode(&delete("tag:prod")), "{\"cache_key\":\"tag:prod\"}");
    let set = Invalidation {
        cache_key: "k".to_owned(),
        new_value: Some(5.0_f64),
        ttl: None,
    };
    assert_eq!(encode(&set), "{\"cache_key\":\"k\",\"new_value\":5}");
    let ttl_only = Invalidation {
        cache_key: "k".to_owned(),
        new_value: None,
        ttl: Some(30.0_f64),
    };
    assert_eq!(encode(&ttl_only), "{\"cache_key\":\"k\",\"ttl\":30}");
    let both = Invalidation {
        cache_key: "k".to_owned(),
        new_value: Some(5.0_f64),
        ttl: Some(30.0_f64),
    };
    assert_eq!(
        encode(&both),
        "{\"cache_key\":\"k\",\"new_value\":5,\"ttl\":30}"
    );
    let explicit = parsed(br#"{"cache_key":"k","new_value":null,"ttl":null}"#);
    assert_eq!(explicit, Some(delete("k")));
}

#[test]
fn escape_control_characters_in_keys() {
    let slash = "\\";
    let key = "key with space\u{1}";
    assert_eq!(
        encode(&delete(key)),
        format!("{{\"cache_key\":\"key with space{slash}u0001\"}}")
    );
}

#[test]
fn utf8_replacement() {
    let invalid_in_key = parsed(b"{\"cache_key\":\"menu\xff\"}");
    let expected = Invalidation {
        cache_key: "menu\u{fffd}".to_owned(),
        new_value: None,
        ttl: None,
    };
    assert_eq!(invalid_in_key, Some(expected.clone()));
    let truncated = parsed(b"{\"cache_key\":\"menu\xe2\x82\"}");
    assert_eq!(truncated, Some(expected));
    assert_eq!(
        decode_invalidation(b"\xff{\"cache_key\":\"k\"}"),
        Err(DecodeError::NotAnObject)
    );
    assert_eq!(
        decode_invalidation(b"{\"cache_key\":\"k\"}\x80"),
        Err(DecodeError::Syntax)
    );
    let valid = parsed("{\"cache_key\":\"caf\u{e9}\"}".as_bytes());
    assert_eq!(
        valid,
        Some(Invalidation {
            cache_key: "caf\u{e9}".to_owned(),
            new_value: None,
            ttl: None,
        })
    );
}

#[test]
fn json_escapes_and_surrogates() {
    let slash = "\\";
    let wire = format!(
        "{}{slash}u00e9 C{slash}u00C9 p{slash}ud83d{slash}ude00\"}}",
        r#"{"cache_key":"q\" b\\ s\/ b\b f\f n\n r\r t\t c"#
    );
    assert_eq!(
        parsed(wire.as_bytes()),
        Some(delete(
            "q\" b\\ s/ b\x08 f\x0c n\n r\r t\t c\u{e9} C\u{c9} p\u{1f600}"
        ))
    );
}

#[test]
fn empty_cache_key() {
    let decoded = parsed(br#"{"cache_key":""}"#);
    assert_eq!(decoded, Some(delete("")));
    assert_eq!(encode(&delete("")), "{\"cache_key\":\"\"}");
    assert_eq!(
        parsed(br#"{"cache_key":"","new_value":0}"#),
        Some(Invalidation {
            cache_key: String::new(),
            new_value: Some(0.0_f64),
            ttl: None,
        })
    );
}

#[test]
fn extra_and_duplicate_fields() {
    let duplicates = br#"{"cache_key":"first","cache_key":"second","new_value":1,"new_value":2,"extra":{"nested":[true,null]}}"#;
    assert_eq!(
        parsed(duplicates),
        Some(Invalidation {
            cache_key: "second".to_owned(),
            new_value: Some(2.0_f64),
            ttl: None,
        })
    );
    let reordered = br#"{"ttl":5,"extra":[true,null,{"a":1}],"cache_key":"k"}"#;
    assert_eq!(
        parsed(reordered),
        Some(Invalidation {
            cache_key: "k".to_owned(),
            new_value: None,
            ttl: Some(5.0_f64),
        })
    );
    let spaced = br#"{"cache_key":"key with space","extra":1}"#;
    assert_eq!(parsed(spaced), Some(delete("key with space")));
}

fn same_number(left: Option<f64>, right: Option<f64>) -> bool {
    match (left, right) {
        (Some(observed), Some(expected)) => observed.to_bits() == expected.to_bits(),
        (None, None) => true,
        (Some(_), None) | (None, Some(_)) => false,
    }
}

#[rstest]
#[case::new_value_bool_true(r#","new_value":true"#, None, None)]
#[case::new_value_bool_false(r#","new_value":false"#, None, None)]
#[case::new_value_string(r#","new_value":"3""#, None, None)]
#[case::new_value_null(r#","new_value":null"#, None, None)]
#[case::new_value_integer(r#","new_value":3"#, Some(3.0_f64), None)]
#[case::new_value_negative(r#","new_value":-0.5"#, Some(-0.5_f64), None)]
#[case::new_value_exponent(r#","new_value":1e3"#, Some(1000.0_f64), None)]
#[case::ttl_bool_true(r#","new_value":1,"ttl":true"#, Some(1.0_f64), None)]
#[case::ttl_string(r#","new_value":1,"ttl":"30""#, Some(1.0_f64), None)]
#[case::ttl_null(r#","new_value":1,"ttl":null"#, Some(1.0_f64), None)]
#[case::ttl_number(r#","new_value":1,"ttl":30"#, Some(1.0_f64), Some(30.0_f64))]
#[case::new_value_nan(r#","new_value":NaN"#, Some(f64::NAN), None)]
#[case::new_value_infinity(r#","new_value":Infinity"#, Some(f64::INFINITY), None)]
#[case::new_value_negative_infinity(r#","new_value":-Infinity"#, Some(f64::NEG_INFINITY), None)]
#[case::new_value_overflow(r#","new_value":1e400"#, Some(f64::INFINITY), None)]
fn numeric_bool_string_null_matrix(
    #[case] suffix: &str,
    #[case] new_value: Option<f64>,
    #[case] ttl: Option<f64>,
) {
    let text = format!("{{\"cache_key\":\"k\"{suffix}}}");
    let decoded = parsed(text.as_bytes()).expect("LiteLLM's parser accepts the payload");
    assert!(same_number(decoded.new_value, new_value), "{suffix}");
    assert!(same_number(decoded.ttl, ttl), "{suffix}");
}

#[test]
fn spend_floor_message() {
    let wire = br#"{"cache_key": "spend:team:1", "new_value": 5.0, "ttl": 30.0}"#;
    let floor = parsed(wire).expect("LiteLLM's floor set decodes");
    assert_eq!(
        floor,
        Invalidation {
            cache_key: "spend:team:1".to_owned(),
            new_value: Some(5.0_f64),
            ttl: Some(30.0_f64),
        }
    );
    let relayed = encode(&floor);
    let round_tripped = parsed(relayed.as_bytes()).expect("our own message decodes");
    assert_eq!(round_tripped, floor);
    assert_eq!(
        relayed,
        "{\"cache_key\":\"spend:team:1\",\"new_value\":5,\"ttl\":30}"
    );
}

#[test]
fn malformed_message() {
    assert_eq!(
        decode_invalidation(b"not json"),
        Err(DecodeError::NotAnObject)
    );
    assert_eq!(decode_invalidation(b""), Err(DecodeError::NotAnObject));
    assert_eq!(
        decode_invalidation(b"\"text\""),
        Err(DecodeError::NotAnObject)
    );
    assert_eq!(decode_invalidation(b"[]"), Err(DecodeError::NotAnObject));
    assert_eq!(decode_invalidation(b"{}"), Ok(TypedValue::new()));
    assert_eq!(read(&TypedValue::new()), None);
    assert_eq!(
        decode_invalidation(br#"{"cache_key":"k",}"#),
        Err(DecodeError::Syntax)
    );
    assert_eq!(
        decode_invalidation(b"{\"cache_key\":\"k\"}trailing"),
        Err(DecodeError::Syntax)
    );
    assert_eq!(
        decode_invalidation(br#"{"cache_key":"k""#),
        Err(DecodeError::Syntax)
    );
}

#[test]
fn malformed_key_values() {
    let number_key = decode_invalidation(br#"{"cache_key":3}"#);
    assert_eq!(number_key.ok().as_ref().and_then(read), None);
    let absent_key = decode_invalidation(br#"{"new_value":1}"#);
    assert_eq!(absent_key.ok().as_ref().and_then(read), None);
    assert_eq!(
        decode_invalidation(br#"{"cache_key":"unterminated}"#),
        Err(DecodeError::Syntax)
    );
    assert_eq!(
        decode_invalidation(br#"{"cache_key":"k","new_value":1.}"#),
        Err(DecodeError::Syntax)
    );
    assert_eq!(
        decode_invalidation(b"{\"cache_key\":\"a\x01b\"}"),
        Err(DecodeError::Syntax)
    );
    let slash = "\\";
    let lone_surrogate = format!("{{\"cache_key\":\"{slash}ud800\"}}");
    assert_eq!(
        decode_invalidation(lone_surrogate.as_bytes()),
        Err(DecodeError::Syntax)
    );
}

#[test]
fn huge_integer_becomes_infinity() {
    let huge = format!("{{\"cache_key\":\"k\",\"new_value\":{}}}", "9".repeat(400));
    let decoded = parsed(huge.as_bytes()).expect("a huge integer still parses");
    assert!(same_number(decoded.new_value, Some(f64::INFINITY)));
}

#[test]
fn message_byte_limit() {
    let mut allowed = String::from("{\"cache_key\":\"k\"}");
    allowed.push_str(&" ".repeat(MAX_MESSAGE_BYTES.saturating_sub(allowed.len())));
    assert_eq!(allowed.len(), MAX_MESSAGE_BYTES);
    let at_limit = decode_invalidation(allowed.as_bytes())
        .expect("a message exactly at the byte limit decodes");
    assert_eq!(
        at_limit.len(),
        1,
        "the whole object is read at the byte limit"
    );
    let mut over = allowed;
    over.push(' ');
    assert_eq!(over.len(), MAX_MESSAGE_BYTES.saturating_add(1));
    assert_eq!(
        decode_invalidation(over.as_bytes()),
        Err(DecodeError::TooLarge {
            length: MAX_MESSAGE_BYTES.saturating_add(1),
            limit: MAX_MESSAGE_BYTES,
        })
    );
    let huge_value = format!("{{\"cache_key\":\"{}\"}}", "x".repeat(MAX_MESSAGE_BYTES));
    assert!(
        matches!(
            decode_invalidation(huge_value.as_bytes()),
            Err(DecodeError::TooLarge { .. })
        ),
        "an oversized value is rejected before parsing"
    );
}

#[test]
fn message_depth_limit() {
    let at_limit = nested_extra(MAX_MESSAGE_DEPTH.saturating_sub(1));
    let decoded = parsed(&at_limit).expect("a message nested to the limit decodes");
    assert_eq!(decoded, delete("k"));
    let over = nested_extra(MAX_MESSAGE_DEPTH);
    assert_eq!(
        decode_invalidation(&over),
        Err(DecodeError::TooDeep {
            limit: MAX_MESSAGE_DEPTH,
        })
    );
    let shallow = nested_extra(1);
    assert_eq!(parsed(&shallow), Some(delete("k")));
    let objects_at_limit = nested_objects(MAX_MESSAGE_DEPTH.saturating_sub(1));
    assert_eq!(parsed(&objects_at_limit), Some(delete("k")));
    let objects_over = nested_objects(MAX_MESSAGE_DEPTH);
    assert_eq!(
        decode_invalidation(&objects_over),
        Err(DecodeError::TooDeep {
            limit: MAX_MESSAGE_DEPTH,
        })
    );
}
