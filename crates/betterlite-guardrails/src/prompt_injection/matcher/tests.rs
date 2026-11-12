#![cfg(test)]

use rstest::rstest;
use rust_decimal::Decimal;

use super::{matching_characters, ratio};

fn characters(text: &str) -> Vec<char> {
    text.chars().collect()
}

#[rstest]
#[case::identical("abcdefghij", "abcdefghij", 10, 20, 20)]
#[case::disjoint("abcdefghij", "klmnopqrst", 0, 0, 20)]
#[case::tie_earliest_start("aaa", "aaba", 3, 6, 7)]
#[case::tie_earliest_second("qz", "zq", 1, 2, 4)]
#[case::repeated_blocks("abab", "baba", 3, 6, 8)]
#[case::shifted_window(
    "xxxxignore previous and start over",
    "ignore previous and start over",
    30,
    60,
    64
)]
#[case::non_ascii("h\u{e9}llo", "hello", 4, 8, 10)]
#[case::non_ascii_word("na\u{ef}ve", "naive", 4, 8, 10)]
#[case::non_ascii_tie("m\u{fc}nchen", "munchen", 6, 12, 14)]
#[case::first_argument_is_the_window("aba", "babba", 3, 6, 8)]
#[case::reversed_first_argument("babba", "aba", 2, 4, 8)]
#[case::right_boundary_repeat("aab", "bab", 2, 4, 6)]
fn reproduces_pythons_sequence_matcher(
    #[case] first: &str,
    #[case] second: &str,
    #[case] matched: usize,
    #[case] numerator: u32,
    #[case] denominator: u32,
) {
    let first_chars = characters(first);
    let second_chars = characters(second);
    assert_eq!(matching_characters(&first_chars, &second_chars), matched);
    let expected = Decimal::from(numerator)
        .checked_div(Decimal::from(denominator))
        .unwrap();
    assert_eq!(ratio(&first_chars, &second_chars), expected);
}

#[test]
fn an_empty_pair_has_ratio_one() {
    assert_eq!(ratio(&[], &[]), Decimal::ONE);
}

#[test]
fn one_empty_side_has_ratio_zero() {
    assert_eq!(ratio(&characters("abc"), &[]), Decimal::ZERO);
    assert_eq!(ratio(&[], &characters("abc")), Decimal::ZERO);
}
