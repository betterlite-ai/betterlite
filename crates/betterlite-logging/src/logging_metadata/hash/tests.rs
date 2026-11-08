#![cfg(test)]

use rstest::rstest;

use super::is_valid_sha256_hash;

#[rstest]
#[case::lowercase("a".repeat(64))]
#[case::uppercase("B".repeat(64))]
#[case::mixed("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_owned())]
fn logging_metadata_behavior_tests_behavior(#[case] value: String) {
    assert!(is_valid_sha256_hash(&value));
}

#[rstest]
#[case::too_short("a".repeat(63))]
#[case::too_long("a".repeat(65))]
#[case::empty(String::new())]
#[case::not_hex("g".repeat(64))]
#[case::raw_key("sk-abcdefghijklmnopqrstuvwxyz0123456789abcdefghijklmnopqrstuvwxyz0123".to_owned())]
fn malformed_values_are_refused(#[case] value: String) {
    assert!(!is_valid_sha256_hash(&value));
}
