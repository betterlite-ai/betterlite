#![cfg(test)]

use super::{hash_token, is_jwt, safe_hash_api_key};
use rstest::rstest;

#[test]
fn bearer_prefix_casefold_once_without_trim() {
    assert_eq!(
        safe_hash_api_key("Bearer sk-test123"),
        hash_token("sk-test123")
    );
    assert_eq!(
        safe_hash_api_key("BEARER sk-test123"),
        hash_token("sk-test123")
    );
    assert_eq!(
        safe_hash_api_key("bearer sk-test123"),
        hash_token("sk-test123")
    );
    // The seven-character prefix check does not trim a second space, so the remainder keeps it and loses the "sk-" match.
    assert_eq!(safe_hash_api_key("Bearer  sk-test123"), " sk-test123");
}

#[test]
fn sk_prefix_case_sensitive() {
    assert_eq!(safe_hash_api_key("SK-test123"), "SK-test123");
    assert_eq!(safe_hash_api_key("sk-test123"), hash_token("sk-test123"));
}

#[test]
fn sk_prefix_before_jwt_shape() {
    assert_eq!(safe_hash_api_key("sk-a.b.c"), hash_token("sk-a.b.c"));
}

#[rstest]
#[case::empty_segments("..")]
#[case::named_segments("a.b.c")]
fn jwt_empty_segments_count(#[case] value: &str) {
    assert!(is_jwt(Some(value)));
    assert_eq!(
        safe_hash_api_key(value),
        format!("hashed-jwt-{}", hash_token(value))
    );
}

#[test]
fn jwt_none_false() {
    assert!(!is_jwt(None));
}

#[test]
fn sha256_utf8_lowercase_hex() {
    assert_eq!(
        hash_token("sk-h\u{e9}llo"),
        "c011e47d8ad63a19a5f03bc4748a0dcc8cebab19f5ef1835e3d0d67eafe037d6"
    );
    assert_eq!(
        hash_token("sk-test123"),
        "bc372bdb48322359f05049dcf298b69067d926cd9f0aa42bb0660de7970b7e29"
    );
}

#[test]
fn already_hashed_identifier_unchanged() {
    let hashed = hash_token("sk-test123");
    assert_eq!(safe_hash_api_key(&hashed), hashed);
}
