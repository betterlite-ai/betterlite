#![cfg(test)]

use rstest::rstest;
use sha2::{Digest as _, Sha256};

use super::{is_jwt, safe_hash_api_key};

#[test]
fn bearer_prefix_casefold_once_without_trim() {
    assert_eq!(
        safe_hash_api_key("Bearer sk-test123"),
        hex::encode(Sha256::digest(b"sk-test123"))
    );
    assert_eq!(
        safe_hash_api_key("BEARER sk-test123"),
        hex::encode(Sha256::digest(b"sk-test123"))
    );
    assert_eq!(
        safe_hash_api_key("bearer sk-test123"),
        hex::encode(Sha256::digest(b"sk-test123"))
    );
    // The seven-character prefix check does not trim a second space, so the remainder keeps it and loses the "sk-" match.
    assert_eq!(safe_hash_api_key("Bearer  sk-test123"), " sk-test123");
}

#[test]
fn sk_prefix_case_sensitive() {
    assert_eq!(safe_hash_api_key("SK-test123"), "SK-test123");
    assert_eq!(
        safe_hash_api_key("sk-test123"),
        hex::encode(Sha256::digest(b"sk-test123"))
    );
}

#[test]
fn sk_prefix_before_jwt_shape() {
    assert_eq!(
        safe_hash_api_key("sk-a.b.c"),
        hex::encode(Sha256::digest(b"sk-a.b.c"))
    );
}

#[rstest]
#[case::empty_segments("..")]
#[case::named_segments("a.b.c")]
fn jwt_empty_segments_count(#[case] value: &str) {
    assert!(is_jwt(Some(value)));
    assert_eq!(
        safe_hash_api_key(value),
        format!(
            "hashed-jwt-{}",
            hex::encode(Sha256::digest(value.as_bytes()))
        )
    );
}

#[rstest]
#[case::two_segments("a.b")]
#[case::four_segments("a.b.c.d")]
fn non_jwt_segment_counts_pass_through(#[case] value: &str) {
    assert!(!is_jwt(Some(value)));
    assert_eq!(safe_hash_api_key(value), value);
}

#[test]
fn double_bearer_prefix_stripped_once() {
    assert_eq!(safe_hash_api_key("Bearer bearer sk-x"), "bearer sk-x");
}

#[test]
fn jwt_none_false() {
    assert!(!is_jwt(None));
}

#[test]
fn sha256_utf8_lowercase_hex() {
    assert_eq!(
        hex::encode(Sha256::digest("sk-h\u{e9}llo".as_bytes())),
        "c011e47d8ad63a19a5f03bc4748a0dcc8cebab19f5ef1835e3d0d67eafe037d6"
    );
    assert_eq!(
        hex::encode(Sha256::digest(b"sk-test123")),
        "bc372bdb48322359f05049dcf298b69067d926cd9f0aa42bb0660de7970b7e29"
    );
}

#[test]
fn already_hashed_identifier_unchanged() {
    let hashed = hex::encode(Sha256::digest(b"sk-test123"));
    assert_eq!(safe_hash_api_key(&hashed), hashed);
}
