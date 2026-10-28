#![cfg(test)]

use super::*;

#[test]
fn json_success_is_empty_object() {
    assert_eq!(
        encode_otlp_response(Some("application/json"), None),
        (b"{}".to_vec(), "application/json".to_owned())
    );
}

#[test]
fn protobuf_error_has_message_field() {
    assert_eq!(
        encode_otlp_response(Some("application/x-protobuf"), Some("bad")).0,
        vec![10, 3, b'b', b'a', b'd']
    );
}

#[test]
fn json_error_and_long_protobuf_error_are_encoded() {
    assert_eq!(
        encode_otlp_response(Some("application/json; charset=utf-8"), Some("bad")),
        (
            br#"{"message":"bad"}"#.to_vec(),
            "application/json".to_owned()
        )
    );
    let long = "x".repeat(130);
    let encoded = encode_otlp_response(None, Some(&long)).0;
    assert_eq!(encoded.first(), Some(&10));
    assert_eq!(encoded.get(1), Some(&130));
}
