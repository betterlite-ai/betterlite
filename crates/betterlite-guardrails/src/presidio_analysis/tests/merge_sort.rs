#![cfg(test)]

use super::shared::item;
use crate::presidio_analysis::merge::merge_chunked_analyze_results;
use crate::presidio_analysis::split::TextChunk;

fn chunk_at(char_offset: usize) -> TextChunk {
    TextChunk {
        char_offset,
        text: String::new(),
    }
}

#[test]
fn merge_sort_orders_the_result_by_start_then_end() {
    let merged = merge_chunked_analyze_results(
        &[chunk_at(0), chunk_at(50)],
        vec![
            vec![
                item("PERSON", 30, 35, 0.9),
                item("EMAIL_ADDRESS", 2, 9, 0.9),
            ],
            vec![
                item("PHONE_NUMBER", 0, 4, 0.9),
                item("LOCATION", 20, 24, 0.9),
            ],
        ],
    );
    let starts: Vec<Option<usize>> = merged.iter().map(|found| found.start).collect();
    assert_eq!(starts, vec![Some(2), Some(30), Some(50), Some(70)]);
}

#[test]
fn merge_sort_breaks_start_ties_by_end() {
    let merged = merge_chunked_analyze_results(
        &[chunk_at(0)],
        vec![vec![
            item("PERSON", 5, 12, 0.9),
            item("EMAIL_ADDRESS", 5, 8, 0.9),
        ]],
    );
    let ends: Vec<Option<usize>> = merged.iter().map(|found| found.end).collect();
    assert_eq!(ends, vec![Some(8), Some(12)]);
}

#[test]
fn merge_sort_lets_different_entity_types_overlap() {
    let merged = merge_chunked_analyze_results(
        &[chunk_at(0)],
        vec![vec![
            item("PERSON", 0, 10, 0.9),
            item("EMAIL_ADDRESS", 3, 8, 0.9),
        ]],
    );
    let types: Vec<Option<&str>> = merged
        .iter()
        .map(|found| found.entity_type.as_deref())
        .collect();
    assert_eq!(types, vec![Some("PERSON"), Some("EMAIL_ADDRESS")]);
}
