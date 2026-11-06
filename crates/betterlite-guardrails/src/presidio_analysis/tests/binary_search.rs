#![cfg(test)]

use rstest::rstest;

use crate::presidio_analysis::split::{
    json_escaped_len, largest_fitting_prefix, split_text_for_analysis,
};

#[rstest]
#[case::all_fit("abcd", 100, 4)]
#[case::ascii_prefix("abcd", 3, 3)]
#[case::exact_fit_with_escape("a\u{e9}", 7, 2)]
#[case::one_byte_short("a\u{e9}", 6, 1)]
#[case::astral_does_not_fit_after_ascii("a\u{1f600}", 12, 1)]
#[case::single_wide_character_is_kept("\u{1f600}a", 5, 1)]
#[case::escapes_count_double("\"\"\"\"", 5, 2)]
fn binary_search_finds_the_largest_prefix_within_the_budget(
    #[case] text: &str,
    #[case] budget: usize,
    #[case] expected: usize,
) {
    let characters: Vec<char> = text.chars().collect();
    assert_eq!(largest_fitting_prefix(&characters, budget), expected);
}

#[test]
fn binary_search_agrees_with_a_linear_scan_for_every_budget() {
    let characters: Vec<char> = "ab\"\u{e9}\u{1f600}cd\n\u{7f}e".chars().collect();
    let linear: Vec<usize> = (1..60)
        .map(|budget| {
            (1..=characters.len())
                .rev()
                .find(|&length| json_escaped_len(characters[..length].iter().copied()) <= budget)
                .unwrap_or(1)
        })
        .collect();
    let searched: Vec<usize> = (1..60)
        .map(|budget| largest_fitting_prefix(&characters, budget))
        .collect();
    assert_eq!(searched, linear);
}

#[test]
fn binary_search_splits_astral_text_one_character_per_chunk_when_each_is_too_wide() {
    let chunks = split_text_for_analysis("\u{1f600}\u{1f600}\u{1f600}", 4, 0);
    let texts: Vec<&str> = chunks.iter().map(|chunk| chunk.text.as_str()).collect();
    assert_eq!(texts, vec!["\u{1f600}", "\u{1f600}", "\u{1f600}"]);
}
