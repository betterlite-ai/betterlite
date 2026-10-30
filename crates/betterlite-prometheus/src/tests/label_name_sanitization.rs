#![cfg(test)]
use betterlite_prometheus::label_sanitization::sanitize_label_name;
use rstest::rstest;

#[rstest]
#[case::digit("9bad-name", "_9bad_name")]
#[case::empty("", "_")]
#[case::valid("a_Z09", "a_Z09")]
#[case::unicode("\u{e9}\u{1f600}a", "__a")]
#[case::punctuation(".a:b c/d", "_a_b_c_d")]
fn replaces_invalid(#[case] input: &str, #[case] expected: &str) {
    assert_eq!(sanitize_label_name(input), expected);
}
