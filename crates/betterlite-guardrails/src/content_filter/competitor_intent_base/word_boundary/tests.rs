#![cfg(test)]

use super::word_boundary_match;
use rstest::rstest;

#[rstest]
#[case::word("compare acme today", "acme", true)]
#[case::substring("acmeish today", "acme", false)]
#[case::case_sensitive("ACME", "acme", false)]
#[case::escaped("a.b", "a.b", true)]
#[case::literal("axb", "a.b", false)]
#[case::unicode("\u{f1}o\u{f1}o airways", "\u{f1}o\u{f1}o", true)]
#[case::underscore("acme_air", "acme", false)]
fn word_boundary(#[case] text: &str, #[case] token: &str, #[case] expected: bool) {
    assert_eq!(word_boundary_match(text, token), expected);
}
