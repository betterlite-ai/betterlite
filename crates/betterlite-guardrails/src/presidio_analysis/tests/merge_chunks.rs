#![cfg(test)]

use serde_json::value::RawValue;

use super::shared::item;
use crate::presidio_analysis::merge::merge_chunked_analyze_results;
use crate::presidio_analysis::split::TextChunk;
use crate::presidio_analysis::wire::AnalyzeResponseItem;

fn chunk_at(char_offset: usize) -> TextChunk {
    TextChunk {
        char_offset,
        text: String::new(),
    }
}

fn spans(items: &[AnalyzeResponseItem]) -> Vec<(Option<usize>, Option<usize>, Option<f64>)> {
    items
        .iter()
        .map(|found| (found.start, found.end, found.score))
        .collect()
}

#[test]
fn merge_chunks_remaps_offsets_to_the_original_text() {
    let merged = merge_chunked_analyze_results(
        &[chunk_at(0), chunk_at(100)],
        vec![
            vec![item("PERSON", 1, 4, 0.8)],
            vec![item("PERSON", 5, 10, 0.9)],
        ],
    );
    assert_eq!(
        spans(&merged),
        vec![
            (Some(1), Some(4), Some(0.8_f64)),
            (Some(105), Some(110), Some(0.9_f64))
        ]
    );
}

#[test]
fn merge_chunks_keeps_the_longest_of_two_overlapping_same_type_spans() {
    let merged = merge_chunked_analyze_results(
        &[chunk_at(0), chunk_at(10)],
        vec![
            vec![item("PERSON", 12, 18, 0.99)],
            vec![item("PERSON", 1, 9, 0.5)],
        ],
    );
    assert_eq!(spans(&merged), vec![(Some(11), Some(19), Some(0.5_f64))]);
}

#[test]
fn merge_chunks_prefers_the_higher_score_when_spans_are_equally_long() {
    let merged = merge_chunked_analyze_results(
        &[chunk_at(0), chunk_at(10)],
        vec![
            vec![item("PERSON", 12, 18, 0.5)],
            vec![item("PERSON", 2, 8, 0.9)],
        ],
    );
    assert_eq!(spans(&merged), vec![(Some(12), Some(18), Some(0.9_f64))]);
}

#[test]
fn merge_chunks_keeps_the_first_of_two_identical_detections() {
    let mut first = item("PERSON", 12, 18, 0.9);
    first.entity_type = Some("PERSON".to_owned());
    let mut second = item("PERSON", 2, 8, 0.9);
    second.other.insert(
        "tag".to_owned(),
        RawValue::from_string("\"second\"".to_owned()).unwrap(),
    );
    let merged = merge_chunked_analyze_results(
        &[chunk_at(0), chunk_at(10)],
        vec![vec![first], vec![second]],
    );
    assert_eq!(merged.len(), 1);
    assert!(merged[0].other.is_empty());
}

#[test]
fn merge_chunks_keeps_same_type_spans_that_only_touch() {
    let merged = merge_chunked_analyze_results(
        &[chunk_at(0)],
        vec![vec![item("PERSON", 0, 5, 0.9), item("PERSON", 5, 9, 0.9)]],
    );
    assert_eq!(merged.len(), 2);
}

#[test]
fn merge_chunks_drops_a_span_nested_inside_a_kept_one() {
    let merged = merge_chunked_analyze_results(
        &[chunk_at(0)],
        vec![vec![item("PERSON", 2, 4, 0.99), item("PERSON", 0, 10, 0.4)]],
    );
    assert_eq!(spans(&merged), vec![(Some(0), Some(10), Some(0.4_f64))]);
}

#[test]
fn merge_chunks_passes_an_empty_chunk_list_through() {
    assert!(merge_chunked_analyze_results(&[], Vec::new()).is_empty());
}
