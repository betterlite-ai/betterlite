#![cfg(test)]

use rstest::rstest;

use crate::presidio_analysis::split::json_escaped_len;

#[rstest]
#[case::plain_ascii("hello", 5)]
#[case::quote_and_backslash("\"\\", 4)]
#[case::named_controls("\n\r\t\u{8}\u{c}", 10)]
#[case::other_controls("\u{0}\u{1f}", 12)]
#[case::delete_character("\u{7f}", 1)]
#[case::latin_one("\u{e9}", 6)]
#[case::bmp_cjk("\u{65e5}", 6)]
#[case::astral_pair("\u{1f600}", 12)]
#[case::empty("", 0)]
fn json_escaped_len_matches_json_dumps_with_ensure_ascii(
    #[case] text: &str,
    #[case] expected: usize,
) {
    assert_eq!(json_escaped_len(text.chars()), expected);
}
