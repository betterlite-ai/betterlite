#![cfg(test)]

use super::text_for_entity_matching;
use rstest::rstest;

#[rstest]
#[case::punctuation("Acme, Inc.!", "Acme Inc")]
#[case::unicode("\u{d1}o\u{f1}o-Airways 42", "\u{d1}o\u{f1}o Airways 42")]
#[case::underscore("our_brand", "our_brand")]
#[case::spaces("\t A   B \n", "A B")]
#[case::empty("!?", "")]
fn entity_text(#[case] input: &str, #[case] expected: &str) {
    assert_eq!(text_for_entity_matching(input), expected);
}
