#![cfg(test)]

use rstest::rstest;

use super::normalize;

#[rstest]
#[case::plain_yes_and_no("a: yes\nb: no", "a: true\nb: false")]
#[case::capitalised_spellings(
    "a: True\nb: FALSE\nc: ON\nd: Off",
    "a: true\nb: false\nc: true\nd: false"
)]
#[case::foreign_casings_are_quoted("a: tRuE\nb: fAlSe", "a: \"tRuE\"\nb: \"fAlSe\"")]
#[case::y_and_n_stay_plain("a: y\nb: n", "a: y\nb: n")]
#[case::foreign_yes_casings_stay_plain("a: yEs\nb: oN", "a: yEs\nb: oN")]
#[case::quoted_spellings_are_left_alone("a: \"yes\"\nb: 'true'", "a: \"yes\"\nb: 'true'")]
#[case::block_scalars_are_left_alone("a: |\n  yes\n  no", "a: |\n  yes\n  no")]
#[case::comments_are_left_alone("a: 1 # yes", "a: 1 # yes")]
#[case::flow_sequences_are_rewritten("a: [yes, Off, tRuE]", "a: [true, false, \"tRuE\"]")]
#[case::keys_are_rewritten("yes: 1", "true: 1")]
#[case::tagged_scalars_are_left_alone("a: !!str yes", "a: !!str yes")]
#[case::multibyte_text_keeps_its_offsets("x: \u{2713}\na: yes", "x: \u{2713}\na: true")]
#[case::nothing_to_rewrite("a: 1\nb: text", "a: 1\nb: text")]
fn normalizes_like_pyyaml(#[case] input: &str, #[case] expected: &str) {
    assert_eq!(normalize(input).as_ref(), expected);
}
