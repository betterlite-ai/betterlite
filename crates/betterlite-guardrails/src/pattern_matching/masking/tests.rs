#![cfg(test)]

use super::{keyword_pattern, mask_spans, redaction_tag};
use std::ops::Range;

use rstest::rstest;

#[rstest]
#[case::metacharacters("a.b+[c](d){e}^$|?\\", r"a\.b\+\[c\]\(d\)\{e\}\^\$\|\?\\")]
#[case::wildcard("se*ret", "se.?ret")]
#[case::escape_space("a # b", "a\\ \\#\\ b")]
fn pattern_matching_behavior(#[case] keyword: &str, #[case] expected: &str) {
    assert_eq!(keyword_pattern(keyword), expected);
}

#[path = "tests/tags/tests.rs"]
mod tags;

#[test]
fn masks_original_unicode_spans_left_to_right() {
    let spans = super::super::spans::merge_spans(vec![3..6, 4..6, 7..10]);
    assert_eq!(
        mask_spans("\u{e9} abc def!", &spans, "$0\\x").unwrap(),
        "\u{e9} $0\\x $0\\x!"
    );
    assert_eq!(mask_spans("abc", &[], "x").unwrap(), "abc");
    mask_spans("\u{e9}", &[Range { start: 1, end: 2 }], "x").unwrap_err();
}

#[path = "tests/spans/tests.rs"]
mod spans;

#[path = "tests/index_errors/tests.rs"]
mod index_errors;

#[path = "tests/decimal_indices/tests.rs"]
mod decimal_indices;
