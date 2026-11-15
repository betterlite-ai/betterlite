#![cfg(test)]

use rstest::rstest;

use super::is_valid_sha256_hash;

const LOWER: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const SHORT: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde";
const UPPER: &str = "0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF";
const MIXED: &str = "0123456789aBcDeF0123456789AbCdEf0123456789aBcDeF0123456789AbCdEf";

#[rstest]
#[case::lowercase(LOWER)]
#[case::uppercase(UPPER)]
#[case::mixed(MIXED)]
fn accepts_exactly_64_ascii_hex_digits(#[case] value: &str) {
    assert!(is_valid_sha256_hash(value));
}

#[rstest]
#[case::empty("")]
#[case::too_short(SHORT)]
#[case::too_long(&format!("{LOWER}0"))]
#[case::non_hex_letter(&format!("{SHORT}g"))]
#[case::punctuation(&format!("{SHORT}-"))]
#[case::space(&format!("{SHORT} "))]
#[case::fullwidth_hex(&format!("{SHORT}ａ"))]
#[case::prefixed(&format!("sha256:{LOWER}"))]
#[case::upper_unicode_digit(&format!("{SHORT}\u{0660}"))]
fn rejects_anything_else(#[case] value: &str) {
    assert!(!is_valid_sha256_hash(value));
}
