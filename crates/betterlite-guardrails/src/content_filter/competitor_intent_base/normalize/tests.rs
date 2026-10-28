#![cfg(test)]

use super::normalize;
use rstest::rstest;

#[rstest]
#[case::normalization("  B4t\u{200b}er  \u{ff23}afe  ", "bater cafe")]
#[case::leet("@473150", "aateiso")]
#[case::zero_width("A\u{200b}c\u{200c}m\u{200d}e\u{2060}\u{feff}", "acme")]
#[case::compatibility("\u{ff21}\u{ff20}\u{ff14}\u{fb03}", "aaaffi")]
#[case::composed("E\u{301}", "\u{e9}")]
#[case::whitespace(" \t\nACME\u{a0}\u{2003}Air\r\n", "acme air")]
#[case::control_separators("\u{1c}ACME\u{1f}Air\u{1d}", "acme air")]
#[case::empty("", "")]
#[case::punctuation("Acme, Inc.!", "acme, inc.!")]
fn normalize_cases(#[case] input: &str, #[case] expected: &str) {
    assert_eq!(normalize(input), expected);
}
