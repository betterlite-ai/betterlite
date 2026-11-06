#![cfg(test)]

use rstest::rstest;

use crate::presidio_analysis::split::split_text_for_analysis;

#[rstest]
#[case::overlap_larger_than_chunk("abcdefghij", 4, 4096)]
#[case::one_character_chunks("abcde", 1, 5)]
#[case::characters_wider_than_the_budget("\u{1f600}\u{1f600}\u{1f600}\u{1f600}\u{1f600}", 4, 100)]
#[case::equal_overlap_and_chunk("abcdefghij", 3, 3)]
fn forward_progress_advances_every_chunk_by_at_least_one_character(
    #[case] text: &str,
    #[case] budget: usize,
    #[case] overlap: usize,
) {
    let chunks = split_text_for_analysis(text, budget, overlap);
    assert!(!chunks.is_empty());
    for pair in chunks.windows(2) {
        assert!(pair[1].char_offset > pair[0].char_offset);
    }
    let last = chunks.last().unwrap();
    assert_eq!(
        last.char_offset.saturating_add(last.text.chars().count()),
        text.chars().count()
    );
}

#[test]
fn forward_progress_gives_a_wide_character_its_own_chunk() {
    let chunks = split_text_for_analysis("\u{1f600}\u{1f600}\u{1f600}", 4, 100);
    let offsets: Vec<usize> = chunks.iter().map(|chunk| chunk.char_offset).collect();
    assert_eq!(offsets, vec![0, 1, 2]);
}
