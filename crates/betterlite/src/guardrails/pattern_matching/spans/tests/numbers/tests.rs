#![cfg(test)]

use super::{convert_word_numbers, rstest};

#[rstest]
#[case::every_digit(
    "zero OH one two three four five six seven eight nine",
    Some("00123456789")
)]
#[case::hyphens("One-TWO-Three", Some("123"))]
#[case::no_tokens("none", Some("1"))]
#[case::empty("", None)]
fn pattern_matching_behavior(#[case] sequence: &str, #[case] expected: Option<&str>) {
    assert_eq!(convert_word_numbers(sequence).unwrap().as_deref(), expected);
}
