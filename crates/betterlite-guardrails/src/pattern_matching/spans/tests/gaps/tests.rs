#![cfg(test)]

use super::{gap_allowed, rstest};

#[rstest]
#[case::empty("", true)]
#[case::whitespace(" \t\n", true)]
#[case::punctuation("?!.", true)]
#[case::unicode_word("\u{63}\u{61}\u{66}\u{e9}", true)]
#[case::unicode_digit("\u{b2}", false)]
#[case::decimal_digit("\u{663}", false)]
#[case::combining_boundary("a\u{345}b", false)]
#[case::fraction_word("\u{bd}", true)]
#[case::two_words("one_two three", false)]
fn pattern_matching_behavior(#[case] gap: &str, #[case] expected: bool) {
    assert_eq!(gap_allowed(gap), expected);
}
