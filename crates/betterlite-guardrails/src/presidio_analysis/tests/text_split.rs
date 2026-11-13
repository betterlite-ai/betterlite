#![cfg(test)]

use rstest::rstest;

use crate::presidio_analysis::split::{TextChunk, json_escaped_len, split_text_for_analysis};

fn offsets_and_lengths(chunks: &[TextChunk]) -> Vec<(usize, usize)> {
    chunks
        .iter()
        .map(|chunk| (chunk.char_offset, chunk.text.chars().count()))
        .collect()
}

#[test]
fn text_split_overlaps_consecutive_chunks() {
    let text = "a".repeat(20);
    let chunks = split_text_for_analysis(&text, 10, 4);
    assert_eq!(
        offsets_and_lengths(&chunks),
        vec![(0, 10), (6, 10), (12, 8)]
    );
}

#[test]
fn text_split_keeps_short_text_in_one_chunk() {
    let chunks = split_text_for_analysis("hello", 10, 4);
    assert_eq!(
        chunks,
        vec![TextChunk {
            char_offset: 0,
            text: "hello".to_owned()
        }]
    );
}

#[test]
fn text_split_returns_no_chunks_for_empty_text() {
    assert!(split_text_for_analysis("", 10, 4).is_empty());
}

#[test]
fn text_split_caps_the_overlap_at_half_the_chunk() {
    let text = "a".repeat(10);
    let chunks = split_text_for_analysis(&text, 4, 4096);
    let offsets: Vec<usize> = chunks.iter().map(|chunk| chunk.char_offset).collect();
    assert_eq!(offsets, vec![0, 2, 4, 6]);
}

#[test]
fn text_split_covers_every_character_of_the_text() {
    let text = "0123456789abcdefghij";
    let chunks = split_text_for_analysis(text, 7, 3);
    let characters: Vec<char> = text.chars().collect();
    let original: Vec<String> = chunks
        .iter()
        .map(|chunk| {
            let length = chunk.text.chars().count();
            characters[chunk.char_offset..chunk.char_offset + length]
                .iter()
                .collect()
        })
        .collect();
    let found: Vec<String> = chunks.iter().map(|chunk| chunk.text.clone()).collect();
    assert_eq!(found, original);
    let last = chunks.last().unwrap();
    assert_eq!(
        last.char_offset + last.text.chars().count(),
        characters.len()
    );
}

#[rstest]
#[case::ascii("abcdefghijklmnopqrstuvwxyz", 8, 2)]
#[case::bmp_non_ascii(&"\u{e9}".repeat(15), 18, 2)]
#[case::astral(&"\u{1f600}".repeat(8), 25, 1)]
#[case::escaped_quotes("\"\"\"\"\"\"\"\"\"\"", 5, 1)]
#[case::mixed("a\n\u{e9}\u{1f600}b\"c\\d\te", 14, 3)]
fn text_split_keeps_each_chunk_within_the_escaped_budget(
    #[case] text: &str,
    #[case] budget: usize,
    #[case] overlap: usize,
) {
    let chunks = split_text_for_analysis(text, budget, overlap);
    assert!(chunks.len() > 1);
    assert!(
        chunks
            .iter()
            .all(|chunk| json_escaped_len(chunk.text.chars()) <= budget)
    );
}

#[test]
fn text_split_counts_escaped_width_not_characters() {
    let chunks = split_text_for_analysis("\u{e9}\u{e9}\u{e9}\u{e9}", 12, 0);
    assert_eq!(offsets_and_lengths(&chunks), vec![(0, 2), (2, 2)]);
}
