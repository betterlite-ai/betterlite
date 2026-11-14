#![cfg(test)]

use rstest::rstest;

use super::is_valid_sha256_hash;

const LOWER: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
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
#[case::too_short(&LOWER[..63])]
#[case::too_long(&format!("{LOWER}0"))]
#[case::non_hex_letter(&format!("{}g", &LOWER[..63]))]
#[case::punctuation(&format!("{}-", &LOWER[..63]))]
#[case::space(&format!("{} ", &LOWER[..63]))]
#[case::fullwidth_hex(&format!("{}ａ", &LOWER[..63]))]
#[case::prefixed(&format!("sha256:{LOWER}"))]
#[case::upper_unicode_digit(&format!("{}{}", &LOWER[..63], '\u{0660}'))]
fn rejects_anything_else(#[case] value: &str) {
    assert!(!is_valid_sha256_hash(value));
}
